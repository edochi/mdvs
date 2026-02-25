use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use mdvs_schema::{FieldInfo, LockFile, Schema, discover_fields, infer_field_paths};
use crate::report::{OutputFormat, format_diagnostics, validate};
use crate::scan::scan_directory;

use super::{lock_path_for, resolve_schema_path};

/// Re-scan `dir` and refresh the lock file.
pub fn cmd_update(dir: &Path, config_arg: Option<&Path>) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let config_path = resolve_schema_path(dir, config_arg)?;

    let schema = Schema::from_file(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;
    let glob = &schema.glob;

    eprintln!("Scanning {} with glob '{}'...", dir.display(), glob);
    let all_files = scan_directory(dir, glob, schema.frontmatter_format)?;

    if all_files.is_empty() {
        bail!("no markdown files found matching '{glob}'");
    }

    let files: Vec<_> = if !schema.include_bare_files {
        all_files
            .into_iter()
            .filter(|f| f.frontmatter.is_some())
            .collect()
    } else {
        all_files
    };
    let total = files.len();
    eprintln!("{} markdown files considered\n", total);

    if total == 0 {
        bail!("no files with frontmatter found (all files are bare)");
    }

    // Validate before updating lock — refuse to snapshot an invalid state
    let diagnostics = validate(&files, &schema);
    if !diagnostics.is_empty() {
        eprint!(
            "{}",
            format_diagnostics(&diagnostics, OutputFormat::Human)
        );
        bail!(
            "{} validation error(s) — lock not updated",
            diagnostics.len()
        );
    }

    // Build inputs for discover_fields: (path, frontmatter) pairs
    let file_frontmatters: Vec<(&str, Option<&serde_json::Value>)> = files
        .iter()
        .map(|f| (f.rel_path.as_str(), f.frontmatter.as_ref()))
        .collect();
    let files_with_frontmatter = file_frontmatters
        .iter()
        .filter(|(_, fm)| fm.is_some())
        .count();
    let field_infos = discover_fields(&file_frontmatters);

    // Build observations for inference
    let observations: Vec<(PathBuf, HashSet<String>)> = files
        .iter()
        .map(|f| {
            let field_names: HashSet<String> = f
                .frontmatter
                .as_ref()
                .and_then(|fm| fm.as_object())
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            (PathBuf::from(&f.rel_path), field_names)
        })
        .collect();
    let _inferred = infer_field_paths(&observations);

    print_field_table(&field_infos, total);

    // Write lock file next to config
    let lock_path = lock_path_for(&config_path);
    let generated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let lock = LockFile::from_discovery(
        &field_infos,
        total,
        files_with_frontmatter,
        glob,
        &generated_at,
    );
    std::fs::write(&lock_path, lock.to_toml_string())
        .with_context(|| format!("failed to write {}", lock_path.display()))?;
    eprintln!("\nWrote {}", lock_path.display());

    Ok(())
}

fn print_field_table(field_infos: &[FieldInfo], total: usize) {
    use comfy_table::{CellAlignment, Table};

    let mut table = Table::new();
    //                    LR TB .--. ....  ......
    table.load_preset("     --            ");
    table.set_header(vec!["Field", "Type", "Count"]);

    if let Some(col) = table.column_mut(2) {
        col.set_cell_alignment(CellAlignment::Right);
    }

    for f in field_infos {
        table.add_row(vec![
            f.name.clone(),
            f.field_type.to_string(),
            format!("{}/{}", f.files.len(), total),
        ]);
    }

    eprintln!("{table}");
}
