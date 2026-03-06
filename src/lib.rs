#![warn(missing_docs)]

//! mdvs — Markdown Validation & Search.
//!
//! A database of markdown documents: schema inference, frontmatter validation,
//! and semantic search with SQL filtering. Single binary, no cloud, no setup.

/// CLI command implementations.
pub mod cmd;
/// File scanning, type inference, and schema discovery.
pub mod discover;
/// Chunking, embedding, storage, and backend abstraction.
pub mod index;
/// Output formatting types and the `CommandOutput` trait.
pub mod output;
/// Configuration file types (`mdvs.toml`) and shared data structures.
pub mod schema;
/// DataFusion-based search context with cosine similarity UDF.
pub mod search;
/// Table rendering helpers (compact and record styles via `tabled`).
pub mod table;
