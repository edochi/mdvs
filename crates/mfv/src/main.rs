use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use mdvs_schema::{FieldDef, FieldType, Schema, auto_promote, discover_fields};
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
    /// Scan markdown files and generate frontmatter.toml
    Init {
        /// Directory to scan
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Output path for frontmatter.toml
        #[arg(long, default_value = "frontmatter.toml")]
        output: PathBuf,

        /// Glob pattern for matching files
        #[arg(long, default_value = "**/*.md")]
        glob: String,

        /// Auto-promote threshold (fraction of files a field must appear in)
        #[arg(long, default_value = "0.5")]
        threshold: f64,

        /// Print discovered fields without writing frontmatter.toml
        #[arg(long)]
        dry_run: bool,
    },

    /// Validate frontmatter against frontmatter.toml
    Check {
        /// Directory to validate
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Path to frontmatter.toml
        #[arg(long, default_value = "frontmatter.toml")]
        schema: PathBuf,

        /// Output format
        #[arg(long, default_value = "human")]
        format: OutputFormat,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Command::Init {
            dir,
            output,
            glob,
            threshold,
            dry_run,
        } => cmd_init(&dir, &output, &glob, threshold, dry_run),
        Command::Check {
            dir,
            schema,
            format,
        } => cmd_check(&dir, &schema, format),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        process::exit(2);
    }
}

fn cmd_init(dir: &Path, output: &Path, glob: &str, threshold: f64, dry_run: bool) -> Result<()> {
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

    // Print field table
    eprintln!(
        "{:<20} {:<12} {:>5}/{:<5} Promoted",
        "Field", "Type", "Count", "Total"
    );
    eprintln!("{}", "-".repeat(56));
    for f in &fields {
        let marker = if f.promoted { "Y" } else { "" };
        eprintln!(
            "{:<20} {:<12} {:>5}/{:<5} {}",
            f.name, f.field_type, f.count, total, marker
        );
    }
    eprintln!();

    if dry_run {
        eprintln!("Dry run — not writing frontmatter.toml");
        return Ok(());
    }

    // Build schema from discovered fields
    let field_defs: Vec<FieldDef> = fields
        .iter()
        .map(|f| {
            let field_type = if f.field_type == FieldType::Enum {
                // Discovery doesn't produce Enum, but just in case
                FieldType::Enum
            } else {
                f.field_type.clone()
            };
            FieldDef {
                name: f.name.clone(),
                field_type,
                required: false,
                paths: vec![],
                pattern: None,
                values: vec![],
                promoted: f.promoted,
            }
        })
        .collect();

    let schema = Schema {
        glob: glob.to_string(),
        fields: field_defs,
    };

    let toml_str = schema.to_toml_string();
    std::fs::write(output, &toml_str)
        .with_context(|| format!("failed to write {}", output.display()))?;
    eprintln!("Wrote {}", output.display());

    Ok(())
}

fn cmd_check(dir: &Path, schema_path: &Path, format: OutputFormat) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let schema = Schema::from_file(schema_path)
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
