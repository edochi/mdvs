use std::path::Path;

use anyhow::Result;
use globset::{Glob, GlobMatcher};
use mdvs_schema::FrontmatterFormat;
use serde_json::Value;
use walkdir::WalkDir;

use super::extract::extract_frontmatter;

/// A scanned markdown file with its extracted frontmatter.
#[derive(Debug)]
pub struct ScannedFile {
    /// Relative path from the scanned directory root.
    pub rel_path: String,
    /// Extracted frontmatter as JSON, or `None` if absent or unparseable.
    pub frontmatter: Option<Value>,
}

/// Walk a directory, extract frontmatter from all matching markdown files.
pub fn scan_directory(
    dir: &Path,
    glob_pattern: &str,
    format: FrontmatterFormat,
) -> Result<Vec<ScannedFile>> {
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

        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }

        if !matcher.is_match(&rel) {
            continue;
        }

        let content = std::fs::read_to_string(path)?;
        let (frontmatter, _body) = extract_frontmatter(&content, format);

        files.push(ScannedFile {
            rel_path: rel,
            frontmatter,
        });
    }

    files.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn finds_markdown_files() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "a.md", "# A");
        write_file(tmp.path(), "b.md", "# B");
        write_file(tmp.path(), "c.md", "# C");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn respects_glob() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "note.md", "# Note");
        write_file(tmp.path(), "readme.txt", "text");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "note.md");
    }

    #[test]
    fn custom_glob() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "blog/post.md", "# Post");
        write_file(tmp.path(), "docs/guide.md", "# Guide");

        let files = scan_directory(tmp.path(), "blog/*", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "blog/post.md");
    }

    #[test]
    fn nested_directory() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "sub/deep/file.md", "# Deep");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "sub/deep/file.md");
    }

    #[test]
    fn empty_directory() {
        let tmp = TempDir::new().unwrap();
        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn extracts_frontmatter() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "note.md", "---\ntitle: Hello\n---\nBody");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        let fm = files[0]
            .frontmatter
            .as_ref()
            .expect("should have frontmatter");
        assert_eq!(fm["title"], serde_json::json!("Hello"));
    }

    #[test]
    fn filters_non_md_files() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "note.md", "---\ntitle: Hello\n---\nBody");
        write_file(tmp.path(), "readme.txt", "text");
        write_file(tmp.path(), "data.json", "{}");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].rel_path, "note.md");
    }

    #[test]
    fn no_frontmatter_is_none() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "plain.md", "Just some text, no delimiters.");

        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Both).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].frontmatter.is_none());
    }

    #[test]
    fn frontmatter_format_filters() {
        let tmp = TempDir::new().unwrap();
        write_file(tmp.path(), "yaml.md", "---\ntitle: Hello\n---\nBody");
        write_file(
            tmp.path(),
            "toml.md",
            "+++\ntitle = \"Hello\"\n+++\nBody",
        );

        // Yaml-only: TOML file treated as bare
        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Yaml).unwrap();
        let yaml_file = files.iter().find(|f| f.rel_path == "yaml.md").unwrap();
        let toml_file = files.iter().find(|f| f.rel_path == "toml.md").unwrap();
        assert!(yaml_file.frontmatter.is_some());
        assert!(toml_file.frontmatter.is_none());

        // Toml-only: YAML file treated as bare
        let files = scan_directory(tmp.path(), "**", FrontmatterFormat::Toml).unwrap();
        let yaml_file = files.iter().find(|f| f.rel_path == "yaml.md").unwrap();
        let toml_file = files.iter().find(|f| f.rel_path == "toml.md").unwrap();
        assert!(yaml_file.frontmatter.is_none());
        assert!(toml_file.frontmatter.is_some());
    }
}
