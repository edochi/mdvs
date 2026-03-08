use clap::{Parser, Subcommand};
use mdvs::output::CommandOutput;
use std::path::PathBuf;

/// Stderr logging level for `--logs`.
#[derive(Clone, clap::ValueEnum)]
enum LogLevel {
    Info,
    Debug,
    Trace,
}

#[derive(Parser)]
#[command(name = "mdvs", about = "Markdown Validation & Search")]
struct Cli {
    /// Output format
    #[arg(short, long, global = true, default_value = "text")]
    output: mdvs::output::OutputFormat,

    /// Show detailed output
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Enable stderr logging
    #[arg(long, global = true, value_name = "LEVEL")]
    logs: Option<LogLevel>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover fields, configure model, write mdvs.toml
    Init {
        /// Directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,
        /// HuggingFace model ID [default: minishlab/potion-base-8M]
        #[arg(long)]
        model: Option<String>,
        /// Pin model to specific revision (commit SHA)
        #[arg(long)]
        revision: Option<String>,
        /// File glob pattern
        #[arg(long, default_value = "**")]
        glob: String,
        /// Overwrite existing config
        #[arg(long)]
        force: bool,
        /// Print discovery table only, write nothing
        #[arg(long)]
        dry_run: bool,
        /// Exclude files without frontmatter
        #[arg(long)]
        ignore_bare_files: bool,
        /// Maximum chunk size in characters [default: 1024]
        #[arg(long)]
        chunk_size: Option<usize>,
        /// Skip building the search index after init
        #[arg(long)]
        suppress_auto_build: bool,
        /// Do not read .gitignore patterns during scan
        #[arg(long)]
        skip_gitignore: bool,
    },
    /// Build or rebuild the search index
    Build {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Change embedding model (requires --force if already configured)
        #[arg(long)]
        set_model: Option<String>,
        /// Change model revision (requires --force if already configured)
        #[arg(long)]
        set_revision: Option<String>,
        /// Change max chunk size (requires --force if already configured)
        #[arg(long)]
        set_chunk_size: Option<usize>,
        /// Confirm config changes that require full re-embed
        #[arg(long)]
        force: bool,
    },
    /// Semantic search across notes
    Search {
        /// Search query
        query: String,
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Maximum number of results
        #[arg(long, short = 'n', default_value = "10")]
        limit: usize,
        /// SQL WHERE clause for filtering
        #[arg(
            long = "where",
            long_help = "SQL WHERE clause for filtering.\n\nExamples:\n  --where \"draft = false\"\n  --where \"tags = 'rust'\"\n  --where \"author = 'O''Brien'\"  (escape ' by doubling)\n\nField names with special characters require SQL quoting:\n  --where \"\\\"author's note\\\" = 'value'\""
        )]
        where_clause: Option<String>,
    },
    /// Validate frontmatter against schema
    Check {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Re-scan and update field definitions
    Update {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Re-infer specific field(s) — can be repeated
        #[arg(long)]
        reinfer: Vec<String>,
        /// Re-infer all fields
        #[arg(long)]
        reinfer_all: bool,
        /// Override auto_build from [update] config
        #[arg(long)]
        build: Option<bool>,
        /// Show what would change, write nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the .mdvs/ directory
    Clean {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Show index status and statistics
    Info {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    if let Some(ref level) = cli.logs {
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let filter = match level {
            LogLevel::Info => "mdvs=info",
            LogLevel::Debug => "mdvs=debug",
            LogLevel::Trace => "mdvs=trace",
        };
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(filter))
            .with(
                tracing_tree::HierarchicalLayer::new(2)
                    .with_targets(false)
                    .with_writer(std::io::stderr)
                    .with_timer(tracing_tree::time::Uptime::default()),
            )
            .init();
    }

    match cli.command {
        Command::Init {
            path,
            model,
            revision,
            glob,
            force,
            dry_run,
            ignore_bare_files,
            chunk_size,
            suppress_auto_build,
            skip_gitignore,
        } => {
            let result = mdvs::cmd::init::run(
                &path,
                model.as_deref(),
                revision.as_deref(),
                &glob,
                force,
                dry_run,
                ignore_bare_files,
                chunk_size,
                !suppress_auto_build,
                skip_gitignore,
                cli.verbose,
            )
            .await?;
            result.print(&cli.output, cli.verbose);
            Ok(())
        }
        Command::Build {
            path,
            set_model,
            set_revision,
            set_chunk_size,
            force,
        } => {
            let outcome = mdvs::cmd::build::run(
                &path,
                set_model.as_deref(),
                set_revision.as_deref(),
                set_chunk_size,
                force,
                cli.verbose,
            )
            .await?;
            match outcome {
                mdvs::cmd::build::BuildOutcome::Success(result) => {
                    result.print(&cli.output, cli.verbose);
                }
                mdvs::cmd::build::BuildOutcome::ValidationFailed(result) => {
                    result.print(&cli.output, cli.verbose);
                    std::process::exit(1);
                }
            }
            Ok(())
        }
        Command::Search {
            query,
            path,
            limit,
            where_clause,
        } => {
            let output =
                mdvs::cmd::search::run(&path, &query, limit, where_clause.as_deref(), cli.verbose)
                    .await;
            output.print(&cli.output, cli.verbose);
            if output.has_failed_step() {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Check { path } => {
            let output = mdvs::cmd::check::run(&path, cli.verbose);
            output.print(&cli.output, cli.verbose);
            if output.has_failed_step() {
                std::process::exit(2);
            }
            if output.result.as_ref().is_some_and(|r| r.has_violations()) {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Update {
            path,
            reinfer,
            reinfer_all,
            build,
            dry_run,
        } => {
            let result =
                mdvs::cmd::update::run(&path, &reinfer, reinfer_all, build, dry_run, cli.verbose)
                    .await?;
            result.print(&cli.output, cli.verbose);
            Ok(())
        }
        Command::Clean { path } => {
            let output = mdvs::cmd::clean::run(&path);
            output.print(&cli.output, cli.verbose);
            if output.has_failed_step() {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Info { path } => {
            let output = mdvs::cmd::info::run(&path, cli.verbose);
            output.print(&cli.output, cli.verbose);
            if output.has_failed_step() {
                std::process::exit(2);
            }
            Ok(())
        }
    }
}
