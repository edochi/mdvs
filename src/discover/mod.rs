//! File discovery, type inference, and frontmatter schema extraction.

/// Recursive field type enum and type widening.
pub mod field_type;
/// Schema inference from scanned files (types, glob patterns, constraints).
pub mod infer;
/// Directory walking and YAML frontmatter parsing.
pub mod scan;
