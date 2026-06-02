//! Phase-by-phase wall-clock profile of the validation + build pipeline.
//!
//! Run with `--release` or numbers will be misleading:
//!
//!     cargo run --release --example profile_pipeline -- <path-to-vault>
//!
//! Two sections of timings:
//!
//! 1. Pre-build phases (the original profile harness):
//!    - walk + read   : enumerate matching files and `read_to_string` each (no parse)
//!    - full scan     : production `ScannedFiles::scan` (walk + read + frontmatter parse)
//!    - parse only    : derived as `full scan - walk-read` (approx; same walker, same reads)
//!    - infer         : `InferredSchema::infer` over the scanned set
//!    - validate      : `cmd::check::validate` over the scanned set
//!
//! 2. Full `build_core` pipeline (scan + auto-update + validate + classify + model
//!    load + embed + write index), with per-step timings extracted from the same
//!    `StepEntry` mechanism the CLI's telemetry uses. This lets us see where the
//!    auto-step overhead actually goes when `search` runs `build_core` internally.

use anyhow::Context;
use globset::Glob;
use ignore::WalkBuilder;
use mdvs::cmd::build::build_core;
use mdvs::cmd::check;
use mdvs::discover::infer::InferredSchema;
use mdvs::discover::scan::ScannedFiles;
use mdvs::outcome::Outcome;
use mdvs::schema::config::MdvsToml;
use mdvs::step::StepEntry;
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

/// Short name for a step's outcome, for table rendering.
fn outcome_label(outcome: &Outcome) -> &'static str {
    match outcome {
        Outcome::Scan(_) => "scan",
        Outcome::Infer(_) => "infer",
        Outcome::WriteConfig(_) => "write_config",
        Outcome::Validate(_) => "validate",
        Outcome::ReadConfig(_) => "read_config",
        Outcome::ReadIndex(_) => "read_index",
        Outcome::Classify(_) => "classify",
        Outcome::LoadModel(_) => "load_model",
        Outcome::EmbedFiles(_) => "embed_files",
        Outcome::WriteIndex(_) => "write_index",
        Outcome::Check(_) => "check",
        Outcome::Init(_) => "init",
        Outcome::Info(_) => "info",
        Outcome::Clean(_) => "clean",
        Outcome::DeleteIndex(_) => "delete_index",
        _ => "other",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: profile_pipeline <path-to-vault>")?;
    let path = std::path::PathBuf::from(path);

    let config_path = path.join("mdvs.toml");
    let config = MdvsToml::read(&config_path)?;
    config.validate()?;

    println!("vault: {}", path.display());
    println!("glob:  {}", config.scan.glob);
    println!();
    println!("== pre-build phases ==");

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

    let total_pre = scan_ms + infer_ms + validate_ms;
    println!();
    println!("pre-build total (scan+infer+validate): {total_pre} ms");

    // ----------------------------------------------------------------------
    // Full build_core: scan → auto-update → validate → classify → model load
    // → embed → write index. Per-step timings come from the same StepEntry
    // mechanism the CLI uses, so we see where the auto-step path actually
    // spends time end-to-end.
    // ----------------------------------------------------------------------
    println!();
    println!("== build_core (auto_update=true, force=false) ==");

    let mut config_mut = MdvsToml::read(&config_path)?;
    let mut steps: Vec<StepEntry> = Vec::new();
    let bc_start = Instant::now();
    let _ = build_core(
        &path,
        &mut config_mut,
        &config_path,
        false, // force
        true,  // auto_update
        &mut steps,
    )
    .await;
    let bc_total_ms = bc_start.elapsed().as_millis();

    let mut sum_ok = 0u64;
    for step in &steps {
        match step {
            StepEntry::Completed(s) => {
                let label = outcome_label(&s.outcome);
                println!("{label:>15} : {:>6} ms", s.elapsed_ms);
                sum_ok += s.elapsed_ms;
            }
            StepEntry::Failed(f) => {
                println!("        FAILED : {:>6} ms  ({})", f.elapsed_ms, f.message);
            }
            StepEntry::Skipped => {
                println!("        skipped : (n/a)");
            }
        }
    }
    let unaccounted = (bc_total_ms as u64).saturating_sub(sum_ok);
    println!();
    println!("build_core total wall : {bc_total_ms} ms");
    println!("sum of step times     : {sum_ok} ms");
    println!("unaccounted (overhead): {unaccounted} ms");

    Ok(())
}
