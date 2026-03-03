#![warn(missing_docs)]

//! mdvs — Markdown Directory Vector Search.
//!
//! Single-binary CLI for semantic search over directories of markdown files.
//! Instant embeddings via Model2Vec, DataFusion + Parquet for storage and search.

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
