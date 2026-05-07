use crate::schema::shared::ScanConfig;
use anyhow::Context;
use globset::Glob;
use gray_matter::engine::YAML;
use gray_matter::{Matter, Pod};
use ignore::WalkBuilder;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, instrument, warn};

const MAX_NESTING_DEPTH: usize = 50;
const MAX_FIELD_COUNT: usize = 1000;

/// Check that a JSON value doesn't exceed the maximum nesting depth.
fn check_depth(val: &Value, max: usize) -> bool {
    fn recurse(val: &Value, depth: usize, max: usize) -> bool {
        if depth > max {
            return false;
        }
        match val {
            Value::Object(map) => map.values().all(|v| recurse(v, depth + 1, max)),
            Value::Array(arr) => arr.iter().all(|v| recurse(v, depth + 1, max)),
            _ => true,
        }
    }
    recurse(val, 0, max)
}

/// A single markdown file with its parsed frontmatter and body content.
#[derive(Debug)]
pub struct ScannedFile {
    /// Path relative to the project root.
    pub path: PathBuf,
    /// Parsed YAML frontmatter as JSON, or `None` for bare files.
    pub data: Option<Value>,
    /// Markdown body (after frontmatter extraction), trimmed.
    pub content: String,
    /// Number of lines before the body in the original file (frontmatter + delimiters).
    /// Used to offset chunk line numbers so they reference the full file.
    pub body_line_offset: usize,
}

/// Collection of scanned markdown files from a directory walk.
#[derive(Debug)]
pub struct ScannedFiles {
    /// All matched files, sorted by relative path.
    pub files: Vec<ScannedFile>,
}

impl ScannedFiles {
    /// Walk a directory, parse frontmatter, and collect matching markdown files.
    #[instrument(name = "scan", skip_all)]
    pub fn scan(root: &Path, config: &ScanConfig) -> anyhow::Result<Self> {
        let matcher = Glob::new(&config.glob)
            .context(format!("invalid glob pattern '{}'", config.glob))?
            .compile_matcher();
        let matter = Matter::<YAML>::new();

        let mut files = Vec::new();

        for entry in WalkBuilder::new(root)
            .hidden(false)
            .add_custom_ignore_filename(".mdvsignore")
            .git_ignore(!config.skip_gitignore)
            .build()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_some_and(|ft| ft.is_file()))
            .filter(|e| {
                e.path()
                    .extension()
                    .is_some_and(|ext| ext == "md" || ext == "markdown")
            })
        {
            let abs_path = entry.path();
            let rel_path = match abs_path.strip_prefix(root) {
                Ok(p) => p.to_path_buf(),
                Err(_) => {
                    warn!(path = %abs_path.display(), "file is outside root directory, skipping");
                    continue;
                }
            };

            if !matcher.is_match(&rel_path) {
                continue;
            }

            // Skip files larger than 100MB to prevent OOM
            const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024;
            match fs::metadata(abs_path) {
                Ok(meta) if meta.len() > MAX_FILE_SIZE => {
                    warn!(
                        path = %rel_path.display(),
                        size_bytes = meta.len(),
                        "file exceeds 100MB limit, skipping"
                    );
                    continue;
                }
                Err(e) => {
                    warn!(path = %abs_path.display(), error = %e, "failed to stat file, skipping");
                    continue;
                }
                _ => {}
            }

            let raw = match fs::read_to_string(abs_path) {
                Ok(content) => content,
                Err(e) => {
                    warn!(path = %abs_path.display(), error = %e, "failed to read file, skipping");
                    continue;
                }
            };
            let Ok(parsed) = matter.parse(&raw) else {
                if !config.include_bare_files {
                    continue;
                }
                files.push(ScannedFile {
                    path: rel_path,
                    data: None,
                    content: raw.trim().to_string(),
                    body_line_offset: 0,
                });
                continue;
            };

            let data = parsed.data.and_then(|d: Pod| {
                let json: Value = d.deserialize().ok()?;
                if json.is_object() { Some(json) } else { None }
            });

            // Safety limits on frontmatter complexity
            if let Some(ref val) = data {
                if let Value::Object(map) = val
                    && map.len() > MAX_FIELD_COUNT
                {
                    warn!(
                        path = %rel_path.display(),
                        fields = map.len(),
                        "frontmatter exceeds {MAX_FIELD_COUNT} fields, skipping"
                    );
                    continue;
                }
                if !check_depth(val, MAX_NESTING_DEPTH) {
                    warn!(
                        path = %rel_path.display(),
                        "frontmatter exceeds {MAX_NESTING_DEPTH} levels of nesting, skipping"
                    );
                    continue;
                }
            }

            if data.is_none() && !config.include_bare_files {
                continue;
            }

            let content = parsed.content.trim().to_string();
            let body_line_offset = raw.lines().count().saturating_sub(content.lines().count());

            files.push(ScannedFile {
                path: rel_path,
                data,
                content,
                body_line_offset,
            });
        }

        files.sort_by(|a, b| a.path.cmp(&b.path));

        info!(files = files.len(), "scan complete");

        Ok(ScannedFiles { files })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_file(root: &Path, rel_path: &str, content: &str) {
        let full = root.join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    fn scan_config(glob: &str, include_bare_files: bool) -> ScanConfig {
        ScanConfig {
            glob: glob.into(),
            include_bare_files,
            skip_gitignore: true,
        }
    }

    fn setup_fixtures(root: &Path) {
        write_file(
            root,
            "blog/post1.md",
            "---\ntitle: Hello\ntags:\n  - rust\n  - arrow\n---\nThis is post 1.",
        );
        write_file(
            root,
            "blog/post2.md",
            "---\ntitle: World\ndraft: true\n---\nThis is post 2.",
        );
        write_file(
            root,
            "blog/drafts/d1.md",
            "---\ntitle: Draft\ncount: 42\n---\nDraft content.",
        );
        write_file(root, "notes/idea.md", "---\ntitle: Idea\n---\nSome idea.");
        write_file(
            root,
            "notes/bare.md",
            "Just plain markdown, no frontmatter.",
        );
        write_file(root, "blog/image.png", "not real png data");
    }

    #[test]
    fn scan_excludes_bare_files() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        assert_eq!(scanned.files.len(), 4);
    }

    #[test]
    fn paths_are_relative_and_sorted() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let paths: Vec<&str> = scanned
            .files
            .iter()
            .map(|f| f.path.to_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            vec![
                "blog/drafts/d1.md",
                "blog/post1.md",
                "blog/post2.md",
                "notes/idea.md",
            ]
        );
    }

    #[test]
    fn frontmatter_parsed() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let post1 = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "blog/post1.md")
            .unwrap();
        let data = post1.data.as_ref().unwrap();
        assert_eq!(data["title"], "Hello");
        assert_eq!(data["tags"][0], "rust");
        assert_eq!(data["tags"][1], "arrow");
    }

    #[test]
    fn content_trimmed_and_excludes_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let post1 = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "blog/post1.md")
            .unwrap();
        assert_eq!(post1.content, "This is post 1.");
    }

    #[test]
    fn boolean_field_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let post2 = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "blog/post2.md")
            .unwrap();
        assert_eq!(post2.data.as_ref().unwrap()["draft"], true);
    }

    #[test]
    fn integer_field_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let d1 = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "blog/drafts/d1.md")
            .unwrap();
        assert_eq!(d1.data.as_ref().unwrap()["count"], 42);
    }

    #[test]
    fn include_bare_files() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", true)).unwrap();
        assert_eq!(scanned.files.len(), 5);
        let bare = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "notes/bare.md")
            .unwrap();
        assert!(bare.data.is_none());
        assert_eq!(bare.content, "Just plain markdown, no frontmatter.");
    }

    #[test]
    fn glob_filtering() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let blog = ScannedFiles::scan(tmp.path(), &scan_config("blog/**", false)).unwrap();
        assert_eq!(blog.files.len(), 3);
        for f in &blog.files {
            assert!(f.path.starts_with("blog/"));
        }

        let notes = ScannedFiles::scan(tmp.path(), &scan_config("notes/**", false)).unwrap();
        assert_eq!(notes.files.len(), 1);
        assert_eq!(notes.files[0].path.to_str().unwrap(), "notes/idea.md");
    }

    #[test]
    fn glob_matches_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("papers/**", false)).unwrap();
        assert_eq!(scanned.files.len(), 0);
    }

    #[test]
    fn non_object_frontmatter_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "scalar.md", "---\njust a string\n---\nBody.");

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", true)).unwrap();
        assert_eq!(scanned.files.len(), 1);
        assert!(scanned.files[0].data.is_none());
    }

    #[test]
    fn nested_object_preserved() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "nested.md",
            "---\ntitle: Nested\nmeta:\n  author: me\n  version: 2\n---\nNested content.",
        );

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let meta = &scanned.files[0].data.as_ref().unwrap()["meta"];
        assert_eq!(meta["author"], "me");
        assert_eq!(meta["version"], 2);
    }

    #[test]
    fn empty_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "empty.md", "---\n---\nBody only.");

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", true)).unwrap();
        assert_eq!(scanned.files.len(), 1);
        if let Some(data) = &scanned.files[0].data {
            assert!(data.as_object().unwrap().is_empty());
        }
    }

    #[test]
    fn content_across_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        setup_fixtures(tmp.path());

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let d1 = scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == "blog/drafts/d1.md")
            .unwrap();
        assert_eq!(d1.content, "Draft content.");
    }

    #[test]
    fn mdvsignore_excludes_files() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "blog/post1.md", "---\ntitle: Hello\n---\nBody.");
        write_file(
            tmp.path(),
            "secret/hidden.md",
            "---\ntitle: Secret\n---\nBody.",
        );
        write_file(tmp.path(), ".mdvsignore", "secret/\n");

        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        assert_eq!(scanned.files.len(), 1);
        assert_eq!(scanned.files[0].path.to_str().unwrap(), "blog/post1.md");
    }
}
