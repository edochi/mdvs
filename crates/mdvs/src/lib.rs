#![warn(missing_docs)]
// Regression gate for TODO-0180: catch new panic-emitters in non-test code
// at PR time. Tests use `.unwrap()` / `.expect()` liberally and are
// excluded via `cfg(not(test))`. Surviving cases in production code must
// carry a local `#[allow(...)]` with a justifying comment.
#![cfg_attr(
    not(test),
    warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! mdvs — Markdown Validation & Search.
//!
//! A database of markdown documents: schema inference, frontmatter validation,
//! and semantic search with SQL filtering. Single binary, no cloud, no setup.

/// Rendering primitives (`Block`, `TableStyle`) and the `Render` trait.
pub mod block;
/// CLI command implementations.
pub mod cmd;
/// File scanning, type inference, and schema discovery.
pub mod discover;
/// Chunking, embedding, storage, and backend abstraction.
pub mod index;
/// Outcome types for all pipeline steps and commands.
pub mod outcome;
/// Output formatting types and the `CommandOutput` trait.
pub mod output;
/// Preprocessor pipeline run before jsonschema validation.
pub mod preprocess;
/// Shared formatters (`format_pretty`, `format_markdown`) that consume `Vec<Block>`.
pub mod render;
/// Configuration file types (`mdvs.toml`) and shared data structures.
pub mod schema;
/// Step tree types for the unified command output architecture.
pub mod step;
/// Table rendering helpers (compact and record styles via `tabled`).
pub mod table;
