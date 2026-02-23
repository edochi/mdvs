use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use mdvs_schema::{FieldDef, Schema, auto_promote, discover_fields};
use mfv::output::{OutputFormat, format_diagnostics};
use mfv::scan::scan_directory;
use mfv::validate::validate;

/// Markdown frontmatter validator.
#[derive(Parser)]
#[command(name = "mfv", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scan markdown files and discover frontmatter fields
    Scan {
        /// Directory to scan
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Glob pattern for matching files
        #[arg(long, default_value = "**/*.md")]
        glob: String,

        /// Auto-promote threshold (fraction of files a field must appear in)
        #[arg(long, default_value = "0.5")]
        threshold: f64,

        /// Write config to file (default: mfv.toml if flag present with no value)
        #[arg(long, num_args = 0..=1, default_missing_value = "mfv.toml")]
        output: Option<PathBuf>,
    },

    /// Validate frontmatter against schema
    Check {
        /// Directory to validate
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Path to schema file (default: auto-discover mfv.toml or mdvs.toml)
        #[arg(long)]
        schema: Option<PathBuf>,

        /// Output format
        #[arg(long, default_value = "human")]
        format: OutputFormat,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Scan {
            dir,
            glob,
            threshold,
            output,
        } => cmd_scan(&dir, &glob, threshold, output.as_deref()),
        Command::Check {
            dir,
            schema,
            format,
        } => cmd_check(&dir, schema.as_deref(), format),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        process::exit(2);
    }
}

fn cmd_scan(dir: &Path, glob: &str, threshold: f64, output: Option<&Path>) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    eprintln!("Scanning {}...", dir.display());
    let files = scan_directory(dir, glob)?;
    let total = files.len();
    eprintln!("Found {} markdown files\n", total);

    if total == 0 {
        bail!("no markdown files found matching '{glob}'");
    }

    let frontmatters: Vec<Option<&serde_json::Value>> =
        files.iter().map(|f| f.frontmatter.as_ref()).collect();
    let mut fields = discover_fields(&frontmatters);
    auto_promote(&mut fields, total, threshold);

    // Print field table to stdout
    println!(
        "{:<20} {:<12} {:>5}/{:<5} Promoted",
        "Field", "Type", "Count", "Total"
    );
    println!("{}", "-".repeat(56));
    for f in &fields {
        let marker = if f.promoted { "Y" } else { "" };
        println!(
            "{:<20} {:<12} {:>5}/{:<5} {}",
            f.name, f.field_type, f.count, total, marker
        );
    }

    if let Some(output_path) = output {
        // Build schema from discovered fields
        let field_defs: Vec<FieldDef> = fields
            .iter()
            .map(|f| FieldDef {
                name: f.name.clone(),
                field_type: f.field_type.clone(),
                required: false,
                paths: vec![],
                pattern: None,
                values: vec![],
                promoted: f.promoted,
            })
            .collect();

        let schema = Schema {
            glob: glob.to_string(),
            fields: field_defs,
            promote_threshold: Some(threshold),
        };

        let toml_str = schema.to_toml_string();
        std::fs::write(output_path, &toml_str)
            .with_context(|| format!("failed to write {}", output_path.display()))?;
        eprintln!("\nWrote {}", output_path.display());
    }

    Ok(())
}

/// Resolve schema path by precedence:
/// 1. Explicit --schema path
/// 2. {dir}/mfv.toml
/// 3. {dir}/mdvs.toml
/// 4. Error
fn resolve_schema_path(dir: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    let mfv_toml = dir.join("mfv.toml");
    if mfv_toml.is_file() {
        return Ok(mfv_toml);
    }

    let mdvs_toml = dir.join("mdvs.toml");
    if mdvs_toml.is_file() {
        return Ok(mdvs_toml);
    }

    bail!(
        "no config found; provide --schema or create mfv.toml / mdvs.toml in {}",
        dir.display()
    )
}

fn cmd_check(dir: &Path, schema_arg: Option<&Path>, format: OutputFormat) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let schema_path = resolve_schema_path(dir, schema_arg)?;

    let schema = Schema::from_file(&schema_path)
        .with_context(|| format!("failed to load schema from {}", schema_path.display()))?;

    let files = scan_directory(dir, &schema.glob)?;
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
