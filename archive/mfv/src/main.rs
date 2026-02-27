//! mfv CLI — markdown frontmatter validator.

use std::path::PathBuf;
use std::process;

use clap::{Parser, Subcommand};

use mdvs_schema::FrontmatterFormat;
use mfv::cmd::{cmd_check, cmd_diff, cmd_init, cmd_update};
use mfv::report::OutputFormat;

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

        /// Include files without frontmatter in analysis
        #[arg(long)]
        include_bare_files: bool,

        /// Omit unconstrained fields from generated config
        #[arg(long)]
        minimal: bool,

        /// Frontmatter format to recognize (yaml, toml, both)
        #[arg(long, default_value = "both")]
        frontmatter_format: FrontmatterFormat,

        /// Date format for inference (chrono strftime syntax, e.g. "%d/%m/%Y")
        #[arg(long)]
        date_format: Option<String>,
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

    /// Compare current directory state against the lock file
    Diff {
        /// Directory to scan
        #[arg(long, default_value = ".")]
        dir: PathBuf,

        /// Path to config file (default: auto-discover mfv.toml or mdvs.toml)
        #[arg(long)]
        config: Option<PathBuf>,

        /// Run diff even if validation fails
        #[arg(long)]
        ignore_validation_errors: bool,
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
            include_bare_files,
            minimal,
            frontmatter_format,
            date_format,
        } => cmd_init(
            &dir,
            &glob,
            &config,
            force,
            dry_run,
            include_bare_files,
            minimal,
            frontmatter_format,
            date_format.as_deref(),
        ),
        Command::Update { dir, config } => cmd_update(&dir, config.as_deref()),
        Command::Check {
            dir,
            schema,
            format,
        } => cmd_check(&dir, schema.as_deref(), format),
        Command::Diff {
            dir,
            config,
            ignore_validation_errors,
        } => cmd_diff(&dir, config.as_deref(), ignore_validation_errors),
    };

    if let Err(e) = result {
        eprintln!("error: {e:#}");
        process::exit(2);
    }
}
