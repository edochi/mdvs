use clap::{Parser, Subcommand};
use mdvs::cmd::init::{InitOptions, InitScanFlags};
use mdvs::cmd::scaffold::ScaffoldCommand;
use mdvs::cmd::search::SearchOptions;
use mdvs::index::backend::SearchQuery;
use mdvs::outcome::commands::export_jsonschema::ExportFormat;
use mdvs::output::OutputFormat;
use mdvs::schema::config::MdvsToml;
use mdvs::step::{CommandResult, has_failed, has_violations};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_tree::HierarchicalLayer;
use tracing_tree::time::Uptime;

/// Stderr logging level for `--logs`.
#[derive(Clone, clap::ValueEnum)]
enum LogLevel {
    Info,
    Debug,
    Trace,
}

#[derive(Parser)]
#[command(name = "mdvs", version, about = "Markdown Validation & Search")]
struct Cli {
    /// Output format. When omitted, falls back to `mdvs.toml`'s
    /// `default_output_format`, then to the hard default `pretty`.
    #[arg(short, long, global = true)]
    output: Option<OutputFormat>,

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
        /// Import fields from a JSON Schema file (.json or .toml).
        /// Skips scan/inference; the file becomes the source of fields.
        #[arg(long = "from-jsonschema", value_name = "PATH")]
        schema: Option<PathBuf>,
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
            long_help = "SQL WHERE clause for filtering.\n\nExamples:\n  --where \"draft = false\"\n  --where \"tags = 'rust'\"                    (array fields — auto-rewritten to array_has)\n  --where \"tags IN ('rust', 'go')\"            (auto-rewritten to OR-chain of array_has)\n  --where \"author = 'O''Brien'\"              (escape ' by doubling)\n\nField names with special characters require SQL quoting:\n  --where \"\\\"author's note\\\" = 'value'\""
        )]
        where_clause: Option<String>,
        /// Retrieval mode: semantic (vector), fulltext (BM25), or hybrid (both)
        #[arg(long, value_enum, default_value_t = mdvs::index::backend::SearchMode::Hybrid)]
        mode: mdvs::index::backend::SearchMode,
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
        /// Override the field definitions with a JSON Schema file.
        /// Replaces the toml's `[fields]` block; if no mdvs.toml exists,
        /// a default config is synthesized. Auto-update is disabled.
        #[arg(long = "jsonschema", value_name = "PATH")]
        schema: Option<PathBuf>,
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
    /// Emit the canonical JSON Schema of mdvs.toml
    ExportJsonschema {
        /// Directory containing mdvs.toml
        #[arg(default_value = ".")]
        path: PathBuf,
        /// Output format
        #[arg(long, value_enum, default_value = "json")]
        format: ExportFormat,
        /// Write to this file instead of stdout
        #[arg(long, value_name = "PATH")]
        output_file: Option<PathBuf>,
    },
    /// Generate install-time artifacts for an agent harness.
    ///
    /// Subcommands emit either the bundled SKILL.md, the project-rules
    /// snippet, or the `PostToolUse` hook config — to stdout, ready to
    /// pipe into the right file under the harness's config dir.
    Scaffold {
        #[command(subcommand)]
        subcommand: ScaffoldCommand,
    },
    /// Agent-harness hook runtime — called by `PostToolUse` hooks.
    ///
    /// Subcommands handle one tool-call payload at a time, reading JSON
    /// from stdin and writing a platform-specific JSON envelope to stdout.
    Hook {
        #[command(subcommand)]
        subcommand: HookCommand,
    },
}

#[derive(Subcommand)]
enum HookCommand {
    /// Handle one hook invocation (reads stdin, writes envelope to stdout).
    Handle {
        /// Platform name (must match a bundled `scaffolding/platforms/<name>/`)
        #[arg(long)]
        platform: String,
        /// Which kind of hook this invocation is — drives the runtime logic.
        #[arg(long, value_enum)]
        kind: mdvs::cmd::hook::HookKind,
    },
}

#[derive(Subcommand)]
enum UpdateCommand {
    /// Re-infer field definitions from scanned files
    Reinfer(mdvs::cmd::update::ReinferArgs),
}

/// Resolve the effective output format from the priority chain:
///
/// 1. `--output` on the CLI (if provided).
/// 2. `default_output_format` in the project's `mdvs.toml` (if present and parseable).
/// 3. Hard fallback: `pretty`.
///
/// A failure to read or parse `mdvs.toml` at step 2 is silent. Config-driven
/// default is a convenience, not load-bearing — diagnostics for malformed
/// configs surface through the command itself when it loads the file.
///
/// TTY autodetection was considered and explicitly rejected: same command
/// should produce same bytes regardless of whether stdout is a terminal, a
/// pipe, or a captured handle (agents, CI). Projects that prefer a
/// different default set `default_output_format` in `mdvs.toml`.
fn resolve_output_format(cli_flag: Option<OutputFormat>, project_path: &Path) -> OutputFormat {
    if let Some(f) = cli_flag {
        return f;
    }
    let toml_path = project_path.join("mdvs.toml");
    if let Ok(config) = MdvsToml::read(&toml_path)
        && let Some(f) = config.default_output_format
    {
        return f;
    }
    OutputFormat::Pretty
}

/// Exit code for a command that failed to run to completion.
const EXIT_FAILURE: u8 = 2;

/// Exit code for a command that ran but found validation violations.
const EXIT_VIOLATIONS: u8 = 1;

/// How [`finish`] turns a command's result into output and an exit code.
#[derive(Clone, Copy)]
enum Finish {
    /// Render the result. Exit with [`EXIT_FAILURE`] on failure, 0 otherwise.
    Plain,
    /// Render the result. Exit with [`EXIT_FAILURE`] on failure,
    /// [`EXIT_VIOLATIONS`] on violations, 0 otherwise.
    ReportViolations,
    /// Render the result only on failure, because on success the command
    /// already wrote its payload to stdout and a trailing summary would make
    /// that output unparseable. Exit codes as for [`Finish::Plain`].
    QuietOnSuccess,
}

/// Render `result` to stdout and map it to the process exit code.
///
/// Output is verbose when `--verbose` is set or the command failed. The
/// format comes from [`resolve_output_format`] for the project at `path`.
fn finish(
    result: &CommandResult,
    path: &Path,
    cli: &Cli,
    mode: Finish,
) -> anyhow::Result<ExitCode> {
    let failed = has_failed(result);
    let render = match mode {
        Finish::Plain | Finish::ReportViolations => true,
        Finish::QuietOnSuccess => failed,
    };
    if render {
        let verbose = cli.verbose || failed;
        let format = resolve_output_format(cli.output, path);
        print!("{}", result.render(&format, verbose)?);
    }
    if failed {
        return Ok(ExitCode::from(EXIT_FAILURE));
    }
    let violations = match mode {
        Finish::ReportViolations => has_violations(result),
        Finish::Plain | Finish::QuietOnSuccess => false,
    };
    if violations {
        return Ok(ExitCode::from(EXIT_VIOLATIONS));
    }
    Ok(ExitCode::SUCCESS)
}

/// Install the hierarchical stderr logger at the level chosen by `--logs`.
fn init_tracing(level: &LogLevel) {
    let filter = match level {
        LogLevel::Info => "mdvs=info",
        LogLevel::Debug => "mdvs=debug",
        LogLevel::Trace => "mdvs=trace",
    };
    tracing_subscriber::registry()
        .with(EnvFilter::new(filter))
        .with(
            HierarchicalLayer::new(2)
                .with_targets(false)
                .with_writer(std::io::stderr)
                .with_timer(Uptime::default()),
        )
        .init();
}

fn run_init(
    cli: &Cli,
    path: &Path,
    glob: &str,
    opts: InitOptions,
    schema: Option<&Path>,
) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::init::run(path, glob, opts, schema, cli.output);
    finish(&result, path, cli, Finish::Plain)
}

async fn run_build(
    cli: &Cli,
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
    no_update: bool,
) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::build::run(
        path,
        set_model,
        set_revision,
        set_chunk_size,
        force,
        no_update,
    )
    .await;
    finish(&result, path, cli, Finish::ReportViolations)
}

async fn run_search(
    cli: &Cli,
    path: &Path,
    query: SearchQuery<'_>,
    opts: SearchOptions,
) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::search::run(path, query, opts).await;
    finish(&result, path, cli, Finish::Plain)
}

fn run_check(
    cli: &Cli,
    path: &Path,
    no_update: bool,
    schema: Option<&Path>,
) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::check::run(path, no_update, cli.verbose, schema);
    finish(&result, path, cli, Finish::ReportViolations)
}

async fn run_update(
    cli: &Cli,
    path: &Path,
    dry_run: bool,
    subcommand: Option<&UpdateCommand>,
) -> anyhow::Result<ExitCode> {
    let reinfer_args = subcommand.map(|UpdateCommand::Reinfer(args)| args);
    let effective_dry_run = dry_run || reinfer_args.is_some_and(|a| a.dry_run);
    let result = mdvs::cmd::update::run(path, reinfer_args, effective_dry_run).await;
    finish(&result, path, cli, Finish::Plain)
}

async fn run_clean(cli: &Cli, path: &Path) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::clean::run(path).await;
    finish(&result, path, cli, Finish::Plain)
}

async fn run_info(cli: &Cli, path: &Path) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::info::run(path).await;
    finish(&result, path, cli, Finish::Plain)
}

fn run_export_jsonschema(
    cli: &Cli,
    path: &Path,
    format: ExportFormat,
    output_file: Option<&Path>,
) -> anyhow::Result<ExitCode> {
    let result = mdvs::cmd::export_jsonschema::run(path, format, output_file);
    // Writing to stdout already emitted the schema, so the summary is
    // suppressed to keep the captured output parseable. Writing to a file
    // prints the human / JSON summary normally.
    let mode = if output_file.is_some() {
        Finish::Plain
    } else {
        Finish::QuietOnSuccess
    };
    finish(&result, path, cli, mode)
}

/// Scaffold subcommands write straight to stdout / stderr and do not
/// produce a `CommandResult`.
fn run_scaffold(subcommand: &ScaffoldCommand) -> anyhow::Result<ExitCode> {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut out = stdout.lock();
    let mut err = stderr.lock();
    match subcommand {
        ScaffoldCommand::Skill { platform } => {
            mdvs::cmd::scaffold::skill::run(&mut out, &mut err, platform.as_deref())?;
        }
        ScaffoldCommand::Snippet { platform } => {
            mdvs::cmd::scaffold::snippet::run(&mut out, &mut err, platform.as_deref())?;
        }
        ScaffoldCommand::Hook { platform } => {
            mdvs::cmd::scaffold::hook::run(&mut out, &mut err, platform)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Hook subcommands write a platform envelope to stdout and do not produce
/// a `CommandResult`.
fn run_hook(subcommand: &HookCommand) -> anyhow::Result<ExitCode> {
    match subcommand {
        HookCommand::Handle { platform, kind } => {
            let stdin = std::io::stdin();
            let mut stdout = std::io::stdout();
            mdvs::cmd::hook::handle::run(stdin.lock(), &mut stdout, platform, *kind)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let cli = Cli::parse();

    if let Some(level) = &cli.logs {
        init_tracing(level);
    }

    match &cli.command {
        Command::Init {
            path,
            glob,
            force,
            dry_run,
            ignore_bare_files,
            skip_gitignore,
            schema,
        } => {
            let opts = InitOptions {
                force: *force,
                dry_run: *dry_run,
                scan: InitScanFlags {
                    ignore_bare_files: *ignore_bare_files,
                    skip_gitignore: *skip_gitignore,
                },
            };
            run_init(&cli, path, glob, opts, schema.as_deref())
        }
        Command::Build {
            path,
            set_model,
            set_revision,
            set_chunk_size,
            force,
            no_update,
        } => {
            run_build(
                &cli,
                path,
                set_model.as_deref(),
                set_revision.as_deref(),
                *set_chunk_size,
                *force,
                *no_update,
            )
            .await
        }
        Command::Search {
            query,
            path,
            limit,
            where_clause,
            mode,
            no_update,
            no_build,
        } => {
            let query = SearchQuery {
                text: query,
                limit: *limit,
                where_clause: where_clause.as_deref(),
                mode: *mode,
            };
            let opts = SearchOptions {
                no_update: *no_update,
                no_build: *no_build,
            };
            run_search(&cli, path, query, opts).await
        }
        Command::Check {
            path,
            no_update,
            schema,
        } => run_check(&cli, path, *no_update, schema.as_deref()),
        Command::Update {
            path,
            dry_run,
            subcommand,
        } => run_update(&cli, path, *dry_run, subcommand.as_ref()).await,
        Command::Clean { path } => run_clean(&cli, path).await,
        Command::Info { path } => run_info(&cli, path).await,
        Command::ExportJsonschema {
            path,
            format,
            output_file,
        } => run_export_jsonschema(&cli, path, *format, output_file.as_deref()),
        Command::Scaffold { subcommand } => run_scaffold(subcommand),
        Command::Hook { subcommand } => run_hook(subcommand),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_config_with_default(dir: &Path, default: &str) {
        let toml = format!(
            r#"default_output_format = "{default}"

[scan]
glob = "**"
include_bare_files = false
skip_gitignore = false
frontmatter_format = "auto"

[fields]
ignore = []
max_categories = 10
min_category_repetition = 3
"#
        );
        fs::write(dir.join("mdvs.toml"), toml).unwrap();
    }

    #[test]
    fn cli_flag_wins_over_config() {
        let dir = TempDir::new().unwrap();
        write_config_with_default(dir.path(), "json");
        let resolved = resolve_output_format(Some(OutputFormat::Markdown), dir.path());
        assert_eq!(resolved, OutputFormat::Markdown);
    }

    #[test]
    fn config_wins_over_hard_default() {
        let dir = TempDir::new().unwrap();
        write_config_with_default(dir.path(), "json");
        let resolved = resolve_output_format(None, dir.path());
        assert_eq!(resolved, OutputFormat::Json);
    }

    #[test]
    fn missing_mdvs_toml_falls_through_to_pretty() {
        let dir = TempDir::new().unwrap();
        let resolved = resolve_output_format(None, dir.path());
        assert_eq!(resolved, OutputFormat::Pretty);
    }

    #[test]
    fn malformed_mdvs_toml_falls_through_silently() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("mdvs.toml"), "this is not valid toml [[[").unwrap();
        // Parse failure on the config peek should NOT propagate — the command
        // itself will surface the error when it tries to load.
        let resolved = resolve_output_format(None, dir.path());
        assert_eq!(resolved, OutputFormat::Pretty);
    }

    #[test]
    fn config_without_default_falls_through_to_pretty() {
        let dir = TempDir::new().unwrap();
        let toml = r#"
[scan]
glob = "**"
include_bare_files = false
skip_gitignore = false
frontmatter_format = "auto"

[fields]
ignore = []
max_categories = 10
min_category_repetition = 3
"#;
        fs::write(dir.path().join("mdvs.toml"), toml).unwrap();
        let resolved = resolve_output_format(None, dir.path());
        assert_eq!(resolved, OutputFormat::Pretty);
    }
}
