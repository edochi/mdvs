#![warn(missing_docs)]

//! mdvs — Markdown Directory Vector Search.
//!
//! Single-binary CLI for semantic search over directories of markdown files.
//! Instant embeddings via Model2Vec, DataFusion + Parquet for storage and search.

pub mod cmd;
pub mod discover;
pub mod index;
pub mod output;
pub mod schema;
pub mod search;
