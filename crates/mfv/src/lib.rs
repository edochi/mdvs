pub mod diagnostic;
pub mod extract;
pub mod output;
pub mod scan;
pub mod validate;

// Re-export the most commonly used function for backwards compat with mdvs crate.
pub use extract::extract_frontmatter;
