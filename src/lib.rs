#![warn(missing_docs)]

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
/// Core pipeline abstractions for structured command output.
pub mod pipeline;
/// Shared formatters (`format_text`, `format_markdown`) that consume `Vec<Block>`.
pub mod render;
/// Configuration file types (`mdvs.toml`) and shared data structures.
pub mod schema;
/// DataFusion-based search context with cosine similarity UDF.
pub mod search;
/// Step tree types for the unified command output architecture.
pub mod step;
/// Table rendering helpers (compact and record styles via `tabled`).
pub mod table;
