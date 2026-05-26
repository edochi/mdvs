use crate::schema::shared::{FrontmatterFormat, ScanConfig};
use anyhow::Context;
use globset::Glob;
use gray_matter::engine::{TOML, YAML};
use gray_matter::{Matter, Pod};
use ignore::WalkBuilder;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, instrument, warn};

const MAX_NESTING_DEPTH: usize = 50;
const MAX_FIELD_COUNT: usize = 1000;

/// Which gray_matter engine to use for a given file. Internal-only; the
/// user-facing equivalent is [`FrontmatterFormat`] (which adds `Auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrontmatterEngine {
    Yaml,
    Toml,
    Json,
}

impl FrontmatterEngine {
    /// Leading delimiter that identifies this engine. Used in
    /// `detect_engine` and in forced-mode mismatch error messages.
    fn delimiter(self) -> &'static str {
        match self {
            Self::Yaml => "---",
            Self::Toml => "+++",
            Self::Json => "{",
        }
    }

    /// Lowercase format name matching `FrontmatterFormat` serde variants.
    /// Used in forced-mode mismatch error messages.
    fn format_name(self) -> &'static str {
        match self {
            Self::Yaml => "yaml",
            Self::Toml => "toml",
            Self::Json => "json",
        }
    }
}

/// Detect the frontmatter engine from the file's first non-empty line.
/// Returns `None` when the file is bare (no recognized delimiter).
fn detect_engine(content: &str) -> Option<FrontmatterEngine> {
    let first = content.lines().find(|l| !l.trim().is_empty())?;
    let trimmed = first.trim_end();
    if trimmed == "---" {
        Some(FrontmatterEngine::Yaml)
    } else if trimmed == "+++" {
        Some(FrontmatterEngine::Toml)
    } else if trimmed.trim_start().starts_with('{') {
        Some(FrontmatterEngine::Json)
    } else {
        None
    }
}

/// Map a [`FrontmatterFormat`] to an explicit [`FrontmatterEngine`], or
/// `None` if the format is `Auto` (caller falls back to `detect_engine`).
fn forced_engine(format: FrontmatterFormat) -> Option<FrontmatterEngine> {
    match format {
        FrontmatterFormat::Auto => None,
        FrontmatterFormat::Yaml => Some(FrontmatterEngine::Yaml),
        FrontmatterFormat::Toml => Some(FrontmatterEngine::Toml),
        FrontmatterFormat::Json => Some(FrontmatterEngine::Json),
    }
}

/// Output of a single engine parse, uniform across YAML / TOML / JSON.
/// `None` means parse failed and the file should fall back to bare-file
/// handling (mirrors gray_matter's `Err` arm).
struct EngineParse {
    data: Option<Value>,
    body: String,
}

/// Parse a file's frontmatter + body with gray_matter using the given
/// `Matter` instance. The `Pod` is converted to `serde_json::Value` and
/// validated (top-level must be an object, NaN/inf rejected). Returns
/// `None` when gray_matter itself fails to parse — caller falls back to
/// bare-file handling for backward compatibility.
fn parse_via_gray_matter<E: gray_matter::engine::Engine>(
    matter: &Matter<E>,
    raw: &str,
) -> Option<(EngineParse, Option<String>)> {
    let parsed = matter.parse::<Pod>(raw).ok()?;
    let (data, error): (Option<Value>, Option<String>) = match parsed.data {
        None => (None, None),
        Some(d) => match d.deserialize::<Value>() {
            Err(e) => (
                None,
                Some(format!("frontmatter not representable as JSON: {e}")),
            ),
            Ok(json) => {
                if json.is_object() {
                    (Some(json), None)
                } else {
                    let kind = json_kind_name(&json);
                    (
                        None,
                        Some(format!("frontmatter must be a key-value map, got {kind}")),
                    )
                }
            }
        },
    };
    Some((
        EngineParse {
            data,
            body: parsed.content,
        },
        error,
    ))
}

/// Parse JSON frontmatter using `serde_json::Deserializer` directly.
/// Hugo-style convention: the JSON object itself starts at column 0 with
/// `{` and ends with the matching `}`; the body follows immediately.
/// gray_matter's `Matter::<JSON>` wraps JSON in `---` delimiters instead,
/// which is not the convention users expect — so we bypass it here.
fn parse_json_native(raw: &str) -> Option<(EngineParse, Option<String>)> {
    // `StreamDeserializer::byte_offset` tells us where the first JSON
    // value ends; the body is everything after.
    let mut iter = serde_json::Deserializer::from_str(raw).into_iter::<Value>();
    match iter.next() {
        Some(Ok(json)) => {
            let consumed = iter.byte_offset();
            let body = raw[consumed..].to_string();
            let (data, error) = if json.is_object() {
                (Some(json), None)
            } else {
                let kind = json_kind_name(&json);
                (
                    None,
                    Some(format!("frontmatter must be a key-value map, got {kind}")),
                )
            };
            Some((EngineParse { data, body }, error))
        }
        Some(Err(e)) => Some((
            EngineParse {
                data: None,
                body: raw.to_string(),
            },
            Some(format!("invalid JSON frontmatter: {e}")),
        )),
        None => None,
    }
}

/// Human-readable name for a JSON value kind, used in `FrontmatterUnrepresentable`
/// error messages when the top-level value isn't an object.
fn json_kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

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
    /// Parsed YAML frontmatter as JSON, or `None` for bare files and for
    /// files whose frontmatter could not be represented (see `frontmatter_error`).
    pub data: Option<Value>,
    /// `Some(reason)` if the file had frontmatter that could not be
    /// converted to JSON (NaN/inf, non-string keys, top-level non-object).
    /// `None` for valid frontmatter and for genuinely bare files.
    pub frontmatter_error: Option<String>,
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

        // Pre-build YAML + TOML Matter instances. Matter::new() defaults
        // delimiter to "---" regardless of engine — TOML must be set
        // explicitly to "+++" per TODO-0162 step 1 spike findings.
        // JSON does not use a gray_matter engine: its convention (Hugo,
        // Astro) is `{...}` where the braces are part of the JSON itself,
        // not delimiters wrapping it. We parse JSON via serde_json
        // directly in `parse_json_native`.
        let yaml_matter = Matter::<YAML>::new();
        let mut toml_matter = Matter::<TOML>::new();
        toml_matter.delimiter = "+++".to_string();

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

            // Resolve which gray_matter engine to use for this file.
            // Auto mode probes the leading delimiter; explicit modes
            // skip the probe but still detect leading delimiters from
            // *other* engines to surface a clear mismatch error.
            let detected = detect_engine(&raw);
            let resolved_engine: Option<FrontmatterEngine> =
                match forced_engine(config.frontmatter_format) {
                    None => detected,
                    Some(forced) => {
                        // Forced-mode mismatch: file's leading delimiter
                        // belongs to a different engine. Surface as
                        // `FrontmatterUnrepresentable` so the user can fix
                        // the file (or relax the config).
                        if let Some(actual) = detected
                            && actual != forced
                        {
                            files.push(ScannedFile {
                                path: rel_path,
                                data: None,
                                frontmatter_error: Some(format!(
                                    "frontmatter format mismatch: configured \
                                 `{}` (delimiter `{}`) but file starts with \
                                 `{}` (delimiter for `{}`)",
                                    forced.format_name(),
                                    forced.delimiter(),
                                    actual.delimiter(),
                                    actual.format_name(),
                                )),
                                content: raw.trim().to_string(),
                                body_line_offset: 0,
                            });
                            continue;
                        }
                        Some(forced)
                    }
                };

            let Some(engine) = resolved_engine else {
                // Bare file (no recognized leading delimiter, or auto
                // mode + empty file). Preserve existing behavior:
                // include or filter based on `include_bare_files`.
                if !config.include_bare_files {
                    continue;
                }
                files.push(ScannedFile {
                    path: rel_path,
                    data: None,
                    frontmatter_error: None,
                    content: raw.trim().to_string(),
                    body_line_offset: 0,
                });
                continue;
            };

            // Dispatch to the engine-specific parser. YAML + TOML go
            // through gray_matter; JSON uses serde_json directly because
            // the `{...}` convention isn't gray_matter's delimiter model.
            // All three branches return a uniform `(EngineParse, error)`
            // so the downstream safety + assembly logic is unified.
            let parsed = match engine {
                FrontmatterEngine::Yaml => parse_via_gray_matter(&yaml_matter, &raw),
                FrontmatterEngine::Toml => parse_via_gray_matter(&toml_matter, &raw),
                FrontmatterEngine::Json => parse_json_native(&raw),
            };
            let Some((parsed, frontmatter_error)) = parsed else {
                if !config.include_bare_files {
                    continue;
                }
                files.push(ScannedFile {
                    path: rel_path,
                    data: None,
                    frontmatter_error: None,
                    content: raw.trim().to_string(),
                    body_line_offset: 0,
                });
                continue;
            };
            let data = parsed.data;

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

            // Bare files (no frontmatter and no error) are filtered when
            // include_bare_files=false. Error files always surface — the
            // user needs to know their YAML is broken.
            if data.is_none() && frontmatter_error.is_none() && !config.include_bare_files {
                continue;
            }

            let content = parsed.body.trim().to_string();
            let body_line_offset = raw.lines().count().saturating_sub(content.lines().count());

            files.push(ScannedFile {
                path: rel_path,
                data,
                frontmatter_error,
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
    use crate::schema::shared::FrontmatterFormat;

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
            frontmatter_format: FrontmatterFormat::Auto,
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

    // ------------------------------------------------------------------------
    // Frontmatter conversion errors (TODO-0149 step 8)
    // ------------------------------------------------------------------------

    fn find<'a>(scanned: &'a ScannedFiles, path: &str) -> &'a ScannedFile {
        scanned
            .files
            .iter()
            .find(|f| f.path.to_str().unwrap() == path)
            .unwrap_or_else(|| panic!("file '{path}' missing from scan"))
    }

    #[test]
    fn scan_captures_top_level_array_as_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "bad.md", "---\n- a\n- b\n---\nBody.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "bad.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("array"), "got: {err}");
    }

    #[test]
    fn scan_captures_top_level_scalar_as_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "scalar.md", "---\nhello\n---\nBody.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "scalar.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("string"), "got: {err}");
    }

    #[test]
    fn scan_captures_top_level_boolean_as_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "bool.md", "---\ntrue\n---\nBody.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "bool.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("boolean"), "got: {err}");
    }

    #[test]
    fn scan_bare_file_has_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "bare.md", "Just markdown.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", true)).unwrap();
        let f = find(&scanned, "bare.md");
        assert!(f.data.is_none());
        assert!(f.frontmatter_error.is_none());
    }

    #[test]
    fn scan_valid_frontmatter_has_no_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "ok.md", "---\ntitle: Hi\n---\nBody.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "ok.md");
        assert!(f.data.is_some());
        assert!(f.frontmatter_error.is_none());
    }

    #[test]
    fn scan_error_file_kept_when_include_bare_files_false() {
        // Error files should always surface, regardless of bare-file filter.
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "bad.md", "---\n- a\n- b\n---\nBody.");
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        assert_eq!(scanned.files.len(), 1);
        assert!(scanned.files[0].frontmatter_error.is_some());
    }

    // ------------------------------------------------------------------------
    // Multi-format frontmatter (TODO-0162)
    // ------------------------------------------------------------------------

    fn scan_config_forced(glob: &str, format: FrontmatterFormat) -> ScanConfig {
        ScanConfig {
            glob: glob.into(),
            include_bare_files: false,
            skip_gitignore: true,
            frontmatter_format: format,
        }
    }

    #[test]
    fn detect_engine_yaml() {
        assert_eq!(
            detect_engine("---\ntitle: x\n---\nbody"),
            Some(FrontmatterEngine::Yaml)
        );
    }

    #[test]
    fn detect_engine_toml() {
        assert_eq!(
            detect_engine("+++\ntitle = \"x\"\n+++\nbody"),
            Some(FrontmatterEngine::Toml)
        );
    }

    #[test]
    fn detect_engine_json() {
        assert_eq!(
            detect_engine("{\n  \"title\": \"x\"\n}\nbody"),
            Some(FrontmatterEngine::Json)
        );
    }

    #[test]
    fn detect_engine_bare() {
        assert_eq!(detect_engine("plain markdown, no frontmatter"), None);
    }

    #[test]
    fn detect_engine_empty() {
        assert_eq!(detect_engine(""), None);
    }

    #[test]
    fn detect_engine_skips_leading_blank_lines() {
        // First non-empty line wins. Whitespace-only lines are skipped.
        assert_eq!(
            detect_engine("\n\n   \n---\ntitle: x\n---\nbody"),
            Some(FrontmatterEngine::Yaml)
        );
    }

    #[test]
    fn detect_engine_dashes_with_trailing_whitespace() {
        // Trailing whitespace on the delimiter line is tolerated.
        assert_eq!(
            detect_engine("---  \ntitle: x\n---\nbody"),
            Some(FrontmatterEngine::Yaml)
        );
    }

    #[test]
    fn scan_parses_toml_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "note.md",
            "+++\ntitle = \"TOML Note\"\ncount = 42\ndraft = false\ntags = [\"alpha\", \"beta\"]\n+++\n\nBody content.",
        );
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "note.md");
        let data = f.data.as_ref().unwrap();
        assert_eq!(data["title"], "TOML Note");
        assert_eq!(data["count"], 42);
        assert_eq!(data["draft"], false);
        assert_eq!(data["tags"][0], "alpha");
        assert_eq!(data["tags"][1], "beta");
        assert_eq!(f.content, "Body content.");
    }

    #[test]
    fn scan_parses_toml_native_date() {
        // Per TODO-0162 step 1 spike: native TOML `Date` and `DateTime`
        // values come through Pod → serde_json::Value as strings.
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "dates.md",
            "+++\njoined = 2024-03-14\nsynced_at = 2024-03-14T10:25:00Z\n+++\n\nBody.",
        );
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "dates.md");
        let data = f.data.as_ref().unwrap();
        assert_eq!(data["joined"], "2024-03-14");
        assert_eq!(data["synced_at"], "2024-03-14T10:25:00Z");
    }

    #[test]
    fn scan_parses_json_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(
            tmp.path(),
            "note.md",
            "{\n  \"title\": \"JSON Note\",\n  \"count\": 7,\n  \"draft\": true\n}\n\nBody content.",
        );
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        let f = find(&scanned, "note.md");
        let data = f.data.as_ref().unwrap();
        assert_eq!(data["title"], "JSON Note");
        assert_eq!(data["count"], 7);
        assert_eq!(data["draft"], true);
        assert_eq!(f.content, "Body content.");
    }

    #[test]
    fn scan_auto_dispatches_mixed_vault() {
        // A single vault with one file per format. Auto mode picks the
        // right engine for each based on the leading delimiter.
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "yaml.md", "---\ntitle: yaml\n---\nyaml body");
        write_file(
            tmp.path(),
            "toml.md",
            "+++\ntitle = \"toml\"\n+++\ntoml body",
        );
        write_file(
            tmp.path(),
            "json.md",
            "{\n  \"title\": \"json\"\n}\njson body",
        );
        let scanned = ScannedFiles::scan(tmp.path(), &scan_config("**", false)).unwrap();
        assert_eq!(scanned.files.len(), 3);
        assert_eq!(
            find(&scanned, "yaml.md").data.as_ref().unwrap()["title"],
            "yaml"
        );
        assert_eq!(
            find(&scanned, "toml.md").data.as_ref().unwrap()["title"],
            "toml"
        );
        assert_eq!(
            find(&scanned, "json.md").data.as_ref().unwrap()["title"],
            "json"
        );
    }

    #[test]
    fn forced_yaml_rejects_toml_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "wrong.md", "+++\ntitle = \"x\"\n+++\nbody");
        let scanned = ScannedFiles::scan(
            tmp.path(),
            &scan_config_forced("**", FrontmatterFormat::Yaml),
        )
        .unwrap();
        let f = find(&scanned, "wrong.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("yaml"), "got: {err}");
        assert!(err.contains("toml"), "got: {err}");
        assert!(err.contains("+++"), "got: {err}");
    }

    #[test]
    fn forced_toml_rejects_yaml_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "wrong.md", "---\ntitle: x\n---\nbody");
        let scanned = ScannedFiles::scan(
            tmp.path(),
            &scan_config_forced("**", FrontmatterFormat::Toml),
        )
        .unwrap();
        let f = find(&scanned, "wrong.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("toml"), "got: {err}");
        assert!(err.contains("yaml"), "got: {err}");
        assert!(err.contains("---"), "got: {err}");
    }

    #[test]
    fn forced_json_rejects_yaml_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "wrong.md", "---\ntitle: x\n---\nbody");
        let scanned = ScannedFiles::scan(
            tmp.path(),
            &scan_config_forced("**", FrontmatterFormat::Json),
        )
        .unwrap();
        let f = find(&scanned, "wrong.md");
        assert!(f.data.is_none());
        let err = f.frontmatter_error.as_ref().unwrap();
        assert!(err.contains("json"), "got: {err}");
        assert!(err.contains("yaml"), "got: {err}");
    }

    #[test]
    fn forced_toml_accepts_toml_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "ok.md", "+++\ntitle = \"x\"\n+++\nbody");
        let scanned = ScannedFiles::scan(
            tmp.path(),
            &scan_config_forced("**", FrontmatterFormat::Toml),
        )
        .unwrap();
        let f = find(&scanned, "ok.md");
        assert_eq!(f.data.as_ref().unwrap()["title"], "x");
        assert!(f.frontmatter_error.is_none());
    }

    #[test]
    fn forced_json_accepts_json_file() {
        let tmp = tempfile::tempdir().unwrap();
        write_file(tmp.path(), "ok.md", "{\n  \"title\": \"x\"\n}\nbody");
        let scanned = ScannedFiles::scan(
            tmp.path(),
            &scan_config_forced("**", FrontmatterFormat::Json),
        )
        .unwrap();
        let f = find(&scanned, "ok.md");
        assert_eq!(f.data.as_ref().unwrap()["title"], "x");
        assert!(f.frontmatter_error.is_none());
    }
}
