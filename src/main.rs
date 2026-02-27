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
    },
    /// Build or rebuild the search index
    Build,
    /// Semantic search across notes
    Search {
        /// Search query
        query: String,
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
        } => mdvs::cmd::init::run(
            &path,
            &model,
            revision.as_deref(),
            &glob,
            force,
            dry_run,
            ignore_bare_files,
        ),
        Command::Build => todo!("build"),
        Command::Search { query: _ } => todo!("search"),
        Command::Check => todo!("check"),
        Command::Update => todo!("update"),
        Command::Clean => todo!("clean"),
        Command::Info => todo!("info"),
    }
}
