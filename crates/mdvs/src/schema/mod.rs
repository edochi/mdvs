//! Configuration and shared data types for `mdvs.toml`.

/// Top-level `mdvs.toml` configuration structure.
pub mod config;
/// Constraint types for field value validation (categorical, range, length).
pub mod constraints;
/// Shared types used across config and discovery (scan, embedding model, chunking, field type serde).
pub mod shared;
