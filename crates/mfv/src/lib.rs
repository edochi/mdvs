//! Markdown frontmatter validator — library for scanning, validating, and reporting
//! on frontmatter fields in markdown files.
#![warn(missing_docs)]

/// Validation diagnostic types.
pub mod diagnostic;
/// Frontmatter extraction from YAML (`---`) and TOML (`+++`) delimited blocks.
pub mod extract;
/// Output formatting for diagnostics (human, JSON, GitHub Actions).
pub mod output;
/// Directory scanning and frontmatter extraction.
pub mod scan;
/// Schema-based frontmatter validation.
pub mod validate;

// Re-export the most commonly used function for backwards compat with mdvs crate.
pub use extract::extract_frontmatter;
