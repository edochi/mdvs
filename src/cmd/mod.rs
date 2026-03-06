//! CLI command implementations.

/// Build the search index (chunk, embed, write Parquet).
pub mod build;
/// Validate frontmatter against the schema.
pub mod check;
/// Delete the `.mdvs/` index directory.
pub mod clean;
/// Display project configuration and index status.
pub mod info;
/// Initialize a new mdvs project.
pub mod init;
/// Query the search index.
pub mod search;
/// Re-scan and update the schema in `mdvs.toml`.
pub mod update;
