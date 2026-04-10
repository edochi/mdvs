use clap::{Parser, Subcommand};
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
    /// Scan a directory, infer a typed schema, and write mdvs.toml
    Init {
        /// Directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,
        /// File glob pattern
        #[arg(long, default_value = "**")]
        glob: String,
        /// Overwrite existing config and delete .mdvs/ if present
        #[arg(long)]
        force: bool,
        /// Print discovery table only, write nothing
        #[arg(long)]
        dry_run: bool,
        /// Exclude files without frontmatter
        #[arg(long)]
        ignore_bare_files: bool,
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
        /// Skip auto-update before building
        #[arg(long)]
        no_update: bool,
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
        /// Skip auto-update before building/searching
        #[arg(long)]
        no_update: bool,
        /// Skip auto-build before searching
        #[arg(long)]
        no_build: bool,
    },
    /// Validate frontmatter against schema
    Check {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Skip auto-update before validating
        #[arg(long)]
        no_update: bool,
    },
    /// Re-scan and update field definitions
    Update {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Show what would change, write nothing
        #[arg(long)]
        dry_run: bool,
        #[command(subcommand)]
        subcommand: Option<UpdateCommand>,
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

#[derive(Subcommand)]
enum UpdateCommand {
    /// Re-infer field definitions from scanned files
    Reinfer(mdvs::cmd::update::ReinferArgs),
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
            glob,
            force,
            dry_run,
            ignore_bare_files,
            skip_gitignore,
        } => {
            let result = mdvs::cmd::init::run(
                &path,
                &glob,
                force,
                dry_run,
                ignore_bare_files,
                skip_gitignore,
                cli.verbose,
            );
            let failed = mdvs::step::has_failed(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Build {
            path,
            set_model,
            set_revision,
            set_chunk_size,
            force,
            no_update,
        } => {
            let result = mdvs::cmd::build::run(
                &path,
                set_model.as_deref(),
                set_revision.as_deref(),
                set_chunk_size,
                force,
                no_update,
                cli.verbose,
            )
            .await;
            let failed = mdvs::step::has_failed(&result);
            let violations = mdvs::step::has_violations(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            if violations {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Search {
            query,
            path,
            limit,
            where_clause,
            no_update,
            no_build,
        } => {
            let result = mdvs::cmd::search::run(
                &path,
                &query,
                limit,
                where_clause.as_deref(),
                no_update,
                no_build,
                cli.verbose,
            )
            .await;
            let failed = mdvs::step::has_failed(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Check { path, no_update } => {
            let result = mdvs::cmd::check::run(&path, no_update, cli.verbose);
            let failed = mdvs::step::has_failed(&result);
            let violations = mdvs::step::has_violations(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            if violations {
                std::process::exit(1);
            }
            Ok(())
        }
        Command::Update {
            path,
            dry_run,
            subcommand,
        } => {
            let reinfer_args = subcommand.map(|UpdateCommand::Reinfer(args)| args);
            let result =
                mdvs::cmd::update::run(&path, reinfer_args.as_ref(), dry_run, cli.verbose).await;
            let failed = mdvs::step::has_failed(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Clean { path } => {
            let result = mdvs::cmd::clean::run(&path);
            let failed = mdvs::step::has_failed(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            Ok(())
        }
        Command::Info { path } => {
            let result = mdvs::cmd::info::run(&path, cli.verbose);
            let failed = mdvs::step::has_failed(&result);
            let verbose = cli.verbose || failed;
            let output_str = match (&cli.output, verbose) {
                (mdvs::output::OutputFormat::Text, true) => {
                    mdvs::render::format_text(&result.render_verbose())
                }
                (mdvs::output::OutputFormat::Text, false) => {
                    mdvs::render::format_text(&result.render_compact())
                }
                (mdvs::output::OutputFormat::Json, true) => {
                    serde_json::to_string_pretty(&result).unwrap()
                }
                (mdvs::output::OutputFormat::Json, false) => match result.result_value() {
                    Some(outcome) => serde_json::to_string_pretty(outcome).unwrap(),
                    None => serde_json::to_string_pretty(&result).unwrap(),
                },
            };
            print!("{output_str}");
            if failed {
                std::process::exit(2);
            }
            Ok(())
        }
    }
}
