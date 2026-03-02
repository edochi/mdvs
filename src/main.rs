use clap::{Parser, Subcommand};
use mdvs::output::CommandOutput;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mdvs", about = "Markdown Directory Vector Search")]
struct Cli {
    /// Output format
    #[arg(short, long, global = true, default_value = "human")]
    output: mdvs::output::OutputFormat,

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
        /// SQL WHERE clause for filtering (e.g. "data['draft'] = false")
        #[arg(long = "where")]
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
    Clean,
    /// Show index status and statistics
    Info,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
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
            )?;
            result.print(&cli.output);
            Ok(())
        }
        Command::Build { path } => mdvs::cmd::build::run(&path),
        Command::Search {
            query,
            path,
            limit,
            where_clause,
        } => {
            mdvs::cmd::search::run(&path, &query, limit, where_clause.as_deref()).await
        }
        Command::Check { path } => {
            let result = mdvs::cmd::check::run(&path)?;
            result.print(&cli.output);
            if result.has_violations() {
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
                mdvs::cmd::update::run(&path, &reinfer, reinfer_all, build, dry_run)?;
            result.print(&cli.output);
            Ok(())
        }
        Command::Clean => todo!("clean"),
        Command::Info => todo!("info"),
    }
}
