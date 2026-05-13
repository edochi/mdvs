//! Configuration and shared data types for `mdvs.toml`.

/// Top-level `mdvs.toml` configuration structure.
pub mod config;
/// Constraint types for field value validation (categorical, range, length).
pub mod constraints;
/// DSL ↔ canonical JSON Schema translation and the mdvs-subset validation gate.
pub(crate) mod json_schema;
/// Schema loading (`load_schema`) and source resolution (`resolve_schema`).
pub(crate) mod load;
/// Shared types used across config and discovery (scan, embedding model, chunking, field type serde).
pub mod shared;
