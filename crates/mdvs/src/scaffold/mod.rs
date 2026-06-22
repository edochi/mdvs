//! Agent-harness scaffolding: per-platform configuration and helpers for the
//! `mdvs scaffold {skill,snippet,hook}` and `mdvs hook handle` subcommands.
//!
//! Per-platform behaviour lives as data, not code: each
//! `scaffolding/platforms/<name>/platform.toml` declares the harness's skill
//! install path, snippet target file + body selection, and (where applicable)
//! the hook config path + event names + matcher patterns. Adding a new
//! harness is a new toml file — no Rust changes, no release for end users.
//!
//! See [`Platform`] for the deserialised shape.

use include_dir::{Dir, include_dir};

/// Bundled scaffolding tree. Embedded at compile time so the production
/// binary needs no disk access to read per-platform configs or the skill /
/// snippet content.
pub static SCAFFOLDING: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/scaffolding");

pub mod platform;
pub mod template;

pub use platform::{HookConfigFormat, HooksConfig, Meta, Platform, SkillConfig, SnippetBody, SnippetConfig};
