//! Phase-by-phase wall-clock profile of the validation pipeline.
//!
//! Run with `--release` or numbers will be misleading:
//!
//!     cargo run --release --example profile_pipeline -- <path-to-vault>
//!
//! Phases:
//!   - walk + read   : enumerate matching files and `read_to_string` each (no parse)
//!   - full scan     : production `ScannedFiles::scan` (walk + read + frontmatter parse)
//!   - parse only    : derived as `full scan - walk-read` (approx; same walker, same reads)
//!   - infer         : `InferredSchema::infer` over the scanned set
//!   - validate      : `cmd::check::validate` over the scanned set

use anyhow::Context;
use globset::Glob;
use ignore::WalkBuilder;
use mdvs::cmd::check;
use mdvs::discover::infer::InferredSchema;
use mdvs::discover::scan::ScannedFiles;
use mdvs::schema::config::MdvsToml;
use std::fs;
use std::path::Path;
use std::time::Instant;

const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;

/// Walk + read every matching markdown file. No frontmatter parsing.
/// Mirrors the filtering in `ScannedFiles::scan` so the timings are
/// apples-to-apples with the full scan.
fn read_only(root: &Path, glob: &str, skip_gitignore: bool) -> anyhow::Result<(usize, usize)> {
    let matcher = Glob::new(glob)?.compile_matcher();
    let mut count = 0usize;
    let mut bytes = 0usize;

    for entry in WalkBuilder::new(root)
        .hidden(false)
        .add_custom_ignore_filename(".mdvsignore")
        .git_ignore(!skip_gitignore)
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
        .filter(|e| {
            e.path()
                .extension()
                .is_some_and(|ext| ext == "md" || ext == "markdown")
        })
    {
        let abs = entry.path();
        let Ok(rel) = abs.strip_prefix(root) else {
            continue;
        };
        if !matcher.is_match(rel) {
            continue;
        }
        match fs::metadata(abs) {
            Ok(meta) if meta.len() > MAX_FILE_SIZE => continue,
            Err(_) => continue,
            _ => {}
        }
        let raw = match fs::read_to_string(abs) {
            Ok(s) => s,
            Err(_) => continue,
        };
        bytes += raw.len();
        count += 1;
    }

    Ok((count, bytes))
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: profile_pipeline <path-to-vault>")?;
    let path = std::path::PathBuf::from(path);

    let config = MdvsToml::read(&path.join("mdvs.toml"))?;
    config.validate()?;

    println!("vault: {}", path.display());
    println!("glob:  {}", config.scan.glob);
    println!();

    let t = Instant::now();
    let (count, bytes) = read_only(&path, &config.scan.glob, config.scan.skip_gitignore)?;
    let read_ms = t.elapsed().as_millis();
    println!(
        "walk + read   : {read_ms:>6} ms   ({count} files, {} KB)",
        bytes / 1024
    );

    let t = Instant::now();
    let scanned = ScannedFiles::scan(&path, &config.scan)?;
    let scan_ms = t.elapsed().as_millis();
    let parse_ms = scan_ms.saturating_sub(read_ms);
    println!(
        "full scan     : {scan_ms:>6} ms   ({} files)",
        scanned.files.len()
    );
    println!("  -> parse    : {parse_ms:>6} ms   (full scan - walk-read)");

    let t = Instant::now();
    let schema = InferredSchema::infer(&scanned);
    let infer_ms = t.elapsed().as_millis();
    println!(
        "infer         : {infer_ms:>6} ms   ({} fields)",
        schema.fields.len()
    );

    let t = Instant::now();
    let result = check::validate(&scanned, &config, false)?;
    let validate_ms = t.elapsed().as_millis();
    println!(
        "validate      : {validate_ms:>6} ms   ({} violations)",
        result.field_violations.len()
    );

    let total = scan_ms + infer_ms + validate_ms;
    println!();
    println!("total (scan+infer+validate): {total} ms");
    Ok(())
}
