use std::path::Path;

use anyhow::Result;
use globset::{Glob, GlobMatcher};
use serde_json::Value;
use walkdir::WalkDir;

use crate::extract::extract_frontmatter;

/// A scanned markdown file with extracted frontmatter.
#[derive(Debug)]
pub struct ScannedFile {
    /// Relative path from the root directory.
    pub rel_path: String,
    /// Extracted frontmatter as JSON, or None if absent/unparseable.
    pub frontmatter: Option<Value>,
}

/// Walk a directory, extract frontmatter from all matching markdown files.
pub fn scan_directory(dir: &Path, glob_pattern: &str) -> Result<Vec<ScannedFile>> {
    let matcher: GlobMatcher = Glob::new(glob_pattern)?.compile_matcher();
    let mut files = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel = path
            .strip_prefix(dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        if !matcher.is_match(&rel) {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let (frontmatter, _body) = extract_frontmatter(&content);

        files.push(ScannedFile {
            rel_path: rel,
            frontmatter,
        });
    }

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}
