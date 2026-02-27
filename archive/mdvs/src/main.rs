use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "mdvs", about = "Semantic search over directories of markdown files")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover fields, configure model, write mdvs.toml + mdvs.lock
    Init {
        /// Target directory (default: current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Model identifier (e.g. "minishlab/potion-base-8M")
        #[arg(long)]
        model: Option<String>,

        /// Glob pattern for file selection
        #[arg(long, default_value = "**")]
        glob: String,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,

        /// Overwrite existing config and lock files
        #[arg(long)]
        force: bool,

        /// Print discovery table only, don't write files
        #[arg(long)]
        dry_run: bool,

        /// Include files without frontmatter
        #[arg(long)]
        include_bare_files: bool,

        /// Minimal config (skip optional fields)
        #[arg(long)]
        minimal: bool,

        /// Frontmatter format to accept
        #[arg(long, default_value = "both")]
        frontmatter_format: String,
    },

    /// Build or rebuild the search index
    Build {
        /// Force full rebuild (ignore content hashes)
        #[arg(long)]
        full: bool,
    },

    /// Search the index
    Search {
        /// Search query
        query: String,

        /// SQL WHERE clause for filtering
        #[arg(long, name = "where")]
        where_clause: Option<String>,

        /// Maximum number of results
        #[arg(short = 'n', long = "limit")]
        limit: Option<usize>,

        /// Output format
        #[arg(long, default_value = "text")]
        format: String,

        /// Show individual chunks instead of files
        #[arg(long)]
        chunks: bool,

        /// Show text snippets from matching chunks
        #[arg(long)]
        snippets: bool,

        /// Build before searching (overrides on_stale config)
        #[arg(long, conflicts_with = "no_build")]
        build: bool,

        /// Skip auto-build even if index is stale
        #[arg(long, conflicts_with = "build")]
        no_build: bool,
    },

    /// Validate files against schema
    Check {
        /// Target directory
        #[arg(long)]
        dir: Option<PathBuf>,

        /// Schema file path
        #[arg(long)]
        schema: Option<PathBuf>,

        /// Output format
        #[arg(long, default_value = "text")]
        format: String,
    },

    /// Re-scan and refresh lock file
    Update {
        /// Target directory
        #[arg(long)]
        dir: Option<PathBuf>,

        /// Config file path
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Remove the .mdvs/ directory
    Clean,

    /// Show index info (model, file count, staleness)
    Info,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Init { .. } => todo!("mdvs init"),
        Command::Build { .. } => todo!("mdvs build"),
        Command::Search { .. } => todo!("mdvs search"),
        Command::Check { .. } => todo!("mdvs check"),
        Command::Update { .. } => todo!("mdvs update"),
        Command::Clean => todo!("mdvs clean"),
        Command::Info => todo!("mdvs info"),
    }
}
