//! Markdown frontmatter validator — library for scanning, validating, and reporting
//! on frontmatter fields in markdown files.
#![warn(missing_docs)]

/// Command implementations: init, update, check, diff.
pub mod cmd;
/// Validation reporting: diagnostics, output formatting, validation logic.
pub mod report;
/// Directory scanning and frontmatter extraction.
pub mod scan;

// Re-export the most commonly used function for backwards compat with mdvs crate.
pub use scan::extract_frontmatter;
