use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use mdvs_schema::{DEFAULT_DATE_FORMATS, FieldDef, FieldInfo, FrontmatterFormat, LockFile, Schema, discover_fields, infer_field_paths};
use crate::scan::scan_directory;

use super::lock_path_for;

/// Discover frontmatter fields in `dir`, write config and lock files.
#[allow(clippy::too_many_arguments)]
pub fn cmd_init(
    dir: &Path,
    glob: &str,
    config_path: &Path,
    force: bool,
    dry_run: bool,
    include_bare_files: bool,
    minimal: bool,
    frontmatter_format: FrontmatterFormat,
    date_format: Option<&str>,
) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    if !dry_run && config_path.exists() && !force {
        bail!(
            "{} already exists (use --force to overwrite)",
            config_path.display()
        );
    }

    eprintln!("Scanning {}...", dir.display());
    let all_files = scan_directory(dir, glob, frontmatter_format)?;

    if all_files.is_empty() {
        bail!("no markdown files found matching '{glob}'");
    }

    let files: Vec<_> = if !include_bare_files {
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

    // Build inputs for discover_fields: (path, frontmatter) pairs
    let file_frontmatters: Vec<(&str, Option<&serde_json::Value>)> = files
        .iter()
        .map(|f| (f.rel_path.as_str(), f.frontmatter.as_ref()))
        .collect();
    let files_with_frontmatter = file_frontmatters
        .iter()
        .filter(|(_, fm)| fm.is_some())
        .count();
    let mut date_fmts: Vec<&str> = DEFAULT_DATE_FORMATS.to_vec();
    if let Some(fmt) = date_format {
        date_fmts.insert(0, fmt);
    }
    let field_infos = discover_fields(&file_frontmatters, &date_fmts);

    // Build observations for inference: all considered files.
    // Bare files (when included) get an empty field set, which correctly
    // prevents fields from being inferred as required at their paths.
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
    let inferred = infer_field_paths(&observations);

    print_field_table(&field_infos, total);

    if dry_run {
        return Ok(());
    }

    // Build schema from discovery + inference
    let mut field_defs: Vec<FieldDef> = field_infos
        .iter()
        .map(|f| {
            let paths = inferred.get(&f.name);
            FieldDef {
                name: f.name.clone(),
                field_type: f.field_type.clone(),
                allowed: paths.map(|p| p.allowed.clone()).unwrap_or_default(),
                required: paths.map(|p| p.required.clone()).unwrap_or_default(),
                pattern: None,
                values: vec![],
                date_format: f.date_format.clone(),
            }
        })
        .collect();

    if minimal {
        field_defs.retain(|f| {
            !(f.allowed == vec!["**".to_string()]
                && f.required.is_empty()
                && f.pattern.is_none()
                && f.values.is_empty())
        });
    }

    let schema = Schema {
        glob: glob.to_string(),
        include_bare_files,
        frontmatter_format,
        fields: field_defs,
    };

    let toml_str = schema.to_toml_string();
    std::fs::write(config_path, &toml_str)
        .with_context(|| format!("failed to write {}", config_path.display()))?;
    eprintln!("\nWrote {}", config_path.display());

    // Write lock file next to config
    let lock_path = lock_path_for(config_path);
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
    eprintln!("Wrote {}", lock_path.display());

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
