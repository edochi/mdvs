//! mfv CLI — markdown frontmatter validator.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use mdvs_schema::{FieldDef, LockFile, Schema, discover_fields, infer_field_paths};
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
    /// Initialize config by scanning markdown files and discovering frontmatter fields
    Init {
        /// Directory to scan
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Glob pattern for matching files
        #[arg(long, default_value = "**")]
        glob: String,

        /// Config file path to write
        #[arg(long, default_value = "mfv.toml")]
        config: PathBuf,

        /// Overwrite existing config
        #[arg(long)]
        force: bool,

        /// Print discovery table only, write nothing
        #[arg(long)]
        dry_run: bool,
    },

    /// Refresh lock file by re-scanning markdown files
    Update {
        /// Directory to scan
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Path to config file (default: auto-discover mfv.toml or mdvs.toml)
        #[arg(long)]
        config: Option<PathBuf>,
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
        Command::Init {
            dir,
            glob,
            config,
            force,
            dry_run,
        } => cmd_init(&dir, &glob, &config, force, dry_run),
        Command::Update { dir, config } => cmd_update(&dir, config.as_deref()),
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

fn cmd_init(
    dir: &Path,
    glob: &str,
    config_path: &Path,
    force: bool,
    dry_run: bool,
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
    let files = scan_directory(dir, glob)?;
    let total = files.len();
    eprintln!("Found {} markdown files\n", total);

    if total == 0 {
        bail!("no markdown files found matching '{glob}'");
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

    // Build observations for inference: (path, set_of_fields) for files with frontmatter
    let observations: Vec<(PathBuf, HashSet<String>)> = files
        .iter()
        .filter_map(|f| {
            let fm = f.frontmatter.as_ref()?;
            let field_names: HashSet<String> = fm
                .as_object()
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            if field_names.is_empty() {
                return None;
            }
            Some((PathBuf::from(&f.rel_path), field_names))
        })
        .collect();
    let inferred = infer_field_paths(&observations);

    print_field_table(&field_infos, total);

    if dry_run {
        return Ok(());
    }

    // Build schema from discovery + inference
    let field_defs: Vec<FieldDef> = field_infos
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
            }
        })
        .collect();

    let schema = Schema {
        glob: glob.to_string(),
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

fn cmd_update(dir: &Path, config_arg: Option<&Path>) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let config_path = resolve_schema_path(dir, config_arg)?;

    let schema = Schema::from_file(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;
    let glob = &schema.glob;

    eprintln!("Scanning {} with glob '{}'...", dir.display(), glob);
    let files = scan_directory(dir, glob)?;
    let total = files.len();
    eprintln!("Found {} markdown files\n", total);

    if total == 0 {
        bail!("no markdown files found matching '{glob}'");
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

    // Build observations for inference (unused for now, but keeps lock consistent with init)
    let observations: Vec<(PathBuf, HashSet<String>)> = files
        .iter()
        .filter_map(|f| {
            let fm = f.frontmatter.as_ref()?;
            let field_names: HashSet<String> = fm
                .as_object()
                .map(|obj| obj.keys().cloned().collect())
                .unwrap_or_default();
            if field_names.is_empty() {
                return None;
            }
            Some((PathBuf::from(&f.rel_path), field_names))
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

/// Derive the lock file path from a config path: `foo.toml` → `foo.lock`.
fn lock_path_for(config_path: &Path) -> PathBuf {
    config_path.with_extension("lock")
}

fn print_field_table(field_infos: &[mdvs_schema::FieldInfo], total: usize) {
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
