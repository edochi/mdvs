//! `mdvs hook` — the runtime that agent-harness PostToolUse hooks call.
//!
//! Reads a hook payload from stdin, runs validate or search-nudge logic,
//! writes a platform-shaped envelope to stdout. Cross-platform because
//! mdvs is a cross-platform Rust binary; no `jq` dependency.
//!
//! Usage from a hook config:
//!
//! ```text
//! mdvs hook handle --platform claude-code --kind validate
//! mdvs hook handle --platform claude-code --kind search-nudge
//! ```
//!
//! See [`handle::run`] for the runtime behaviour, and `scaffold::Platform`
//! for the per-platform data this command reads.

pub mod handle;

/// Which kind of hook is being handled.
///
/// Closed enum because the set is small and stable: `validate` runs
/// `mdvs check` after an edit; `search-nudge` emits a one-line tip when
/// the agent runs grep/rg/find/etc. inside an mdvs vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum HookKind {
    /// Run after an Edit/Write/MultiEdit tool call: validates frontmatter
    /// in the vault containing the edited file and surfaces violations
    /// (non-blocking).
    Validate,
    /// Run after a Bash tool call: if the command is a search tool (grep,
    /// rg, find, fd, ag, ack, git grep) and the agent's cwd is inside an
    /// mdvs vault, emit a one-line tip pointing at `mdvs search`.
    SearchNudge,
}
