use std::path::Path;
use std::process;

use anyhow::{Context, Result, bail};

use mdvs_schema::Schema;
use crate::report::{OutputFormat, format_diagnostics, validate};
use crate::scan::scan_directory;

use super::resolve_schema_path;

/// Validate frontmatter in `dir` against a schema file.
pub fn cmd_check(dir: &Path, schema_arg: Option<&Path>, format: OutputFormat) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let schema_path = resolve_schema_path(dir, schema_arg)?;

    let schema = Schema::from_file(&schema_path)
        .with_context(|| format!("failed to load schema from {}", schema_path.display()))?;

    let all_files = scan_directory(dir, &schema.glob, schema.frontmatter_format)?;
    let files: Vec<_> = if !schema.include_bare_files {
        all_files
            .into_iter()
            .filter(|f| f.frontmatter.is_some())
            .collect()
    } else {
        all_files
    };
    eprintln!(
        "Checking {} files against {}\n",
        files.len(),
        schema_path.display()
    );

    let diagnostics = validate(&files, &schema);

    if diagnostics.is_empty() {
        if format == OutputFormat::Json {
            println!("[]");
        } else {
            println!("All files valid.");
        }
        process::exit(0);
    }

    print!("{}", format_diagnostics(&diagnostics, format));
    process::exit(1);
}
