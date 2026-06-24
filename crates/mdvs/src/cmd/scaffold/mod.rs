//! `mdvs scaffold` — install-time generator commands.
//!
//! Three subcommands, all reading from the bundled
//! [`crate::scaffold::SCAFFOLDING`] tree and the per-platform
//! [`crate::scaffold::Platform`] config:
//!
//! - [`skill`] — print the bundled `SKILL.md`. Pipe to the harness's skill
//!   directory.
//! - [`snippet`] — print the project-rules snippet. Pipe / append to
//!   `CLAUDE.md` / `AGENTS.md` / `.cursor/rules/mdvs.mdc`.
//! - [`hook`] — print the harness's PostToolUse hook config. Merge into
//!   `.claude/settings.json` / `.codex/hooks.json` / `.cursor/hooks.json`.
//!
//! Each subcommand writes pure body content to stdout, so `mdvs scaffold
//! skill > .claude/skills/mdvs/SKILL.md` works without polluting the file.
//! Install hints are printed to stderr (and visible interactively but
//! invisible to a redirect).

pub mod hook;
pub mod skill;
pub mod snippet;

/// `mdvs scaffold` subcommand.
#[derive(Debug, clap::Subcommand)]
pub enum ScaffoldCommand {
    /// Print the bundled mdvs skill file.
    ///
    /// Default: prints the universal `SKILL.md`. With `--platform`, also
    /// emits an install-path hint on stderr (the body on stdout is
    /// identical — pipe-safe).
    Skill {
        /// Target harness (claude-code, codex, cursor, opencode, antigravity).
        #[arg(long)]
        platform: Option<String>,
    },
    /// Print the project-rules snippet for the agent.
    ///
    /// Default: prints the universal `AGENTS.md`-flavoured block. With
    /// `--platform`, picks the platform's preferred body (e.g. Cursor
    /// uses the `.mdc`-wrapped variant for `.cursor/rules/`).
    Snippet {
        /// Target harness.
        #[arg(long)]
        platform: Option<String>,
    },
    /// Print the per-platform PostToolUse hook config (JSON) to merge into
    /// the harness's hooks file.
    ///
    /// `--platform` is required because the JSON shape varies (config file
    /// path, event-name capitalization). The emitted config calls
    /// `mdvs hook handle --platform <name> --kind <kind>` for each matcher
    /// — no shell scripts to install.
    ///
    /// Refuses for platforms without a shell-hook surface (OpenCode,
    /// Antigravity).
    Hook {
        /// Target harness.
        #[arg(long)]
        platform: String,
    },
}
