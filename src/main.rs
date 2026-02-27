use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "mdvs", about = "Markdown Directory Vector Search")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Discover fields, write config + lock, build index
    Init,
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
async fn main() {
    let cli = Cli::parse();
    match cli.command {
        Command::Init => todo!("init"),
        Command::Build => todo!("build"),
        Command::Search { query: _ } => todo!("search"),
        Command::Check => todo!("check"),
        Command::Update => todo!("update"),
        Command::Clean => todo!("clean"),
        Command::Info => todo!("info"),
    }
}
