use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "mdvs", about = "Markdown Directory Vector Search")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover fields, configure model, write mdvs.toml + mdvs.lock
    Init {
        /// Directory to scan
        #[arg(default_value = ".")]
        path: PathBuf,
        /// HuggingFace model ID
        #[arg(long, default_value = "minishlab/potion-base-8M")]
        model: String,
        /// Pin model to specific revision (commit SHA)
        #[arg(long)]
        revision: Option<String>,
        /// File glob pattern
        #[arg(long, default_value = "**")]
        glob: String,
        /// Overwrite existing config and lock files
        #[arg(long)]
        force: bool,
        /// Print discovery table only, write nothing
        #[arg(long)]
        dry_run: bool,
        /// Exclude files without frontmatter
        #[arg(long)]
        ignore_bare_files: bool,
        /// Maximum chunk size in characters
        #[arg(long, default_value = "1024")]
        chunk_size: usize,
        /// Automatically build index after update
        #[arg(long, default_value = "true")]
        auto_build: bool,
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
        #[arg(long, name = "where")]
        where_clause: Option<String>,
    },
    /// Validate frontmatter against schema
    Check,
    /// Re-scan and refresh lock file
    Update,
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
            auto_build,
        } => mdvs::cmd::init::run(
            &path,
            &model,
            revision.as_deref(),
            &glob,
            force,
            dry_run,
            ignore_bare_files,
            chunk_size,
            auto_build,
        ),
        Command::Build { path } => mdvs::cmd::build::run(&path),
        Command::Search {
            query,
            path,
            limit,
            where_clause,
        } => {
            mdvs::cmd::search::run(&path, &query, limit, where_clause.as_deref()).await
        }
        Command::Check => todo!("check"),
        Command::Update => todo!("update"),
        Command::Clean => todo!("clean"),
        Command::Info => todo!("info"),
    }
}
