use crate::discover::scan::ScannedFiles;
use crate::index::backend::Backend;
use crate::output::{field_hints, format_hints, CommandOutput, FieldHint};
use crate::schema::config::MdvsToml;
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

/// A field definition from `mdvs.toml`, rendered for display.
#[derive(Debug, Serialize)]
pub struct InfoField {
    /// Field name.
    pub name: String,
    /// Inferred or configured type (e.g. `"String"`, `"Boolean"`, `"String[]"`).
    pub field_type: String,
    /// Glob patterns where this field may appear.
    pub allowed: Vec<String>,
    /// Glob patterns where this field must appear.
    pub required: Vec<String>,
    /// Number of files containing this field (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    /// Total scanned files for computing prevalence (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_files: Option<usize>,
    /// Hints about special characters in the field name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<FieldHint>,
}

/// Metadata and statistics about a built search index.
#[derive(Debug, Serialize)]
pub struct IndexInfo {
    /// Embedding model name.
    pub model: String,
    /// Pinned model revision (commit SHA), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// Maximum chunk size in characters.
    pub chunk_size: usize,
    /// Number of files in the index.
    pub files_indexed: usize,
    /// Number of files on disk (for N/N display).
    pub files_on_disk: usize,
    /// Total number of chunks across all files.
    pub chunks: usize,
    /// ISO 8601 timestamp of last build.
    pub built_at: String,
    /// Config status: `"match"` or `"changed — rebuild recommended"`.
    pub config_status: String,
}

/// Output of the `info` command.
#[derive(Debug, Serialize)]
pub struct InfoResult {
    /// Glob pattern from `[scan]` config.
    pub scan_glob: String,
    /// Number of markdown files matching the scan pattern.
    pub files_on_disk: usize,
    /// Field definitions from `[[fields.field]]`.
    pub fields: Vec<InfoField>,
    /// Field names in the `[fields].ignore` list.
    pub ignored_fields: Vec<String>,
    /// Index info, if a built index exists.
    pub index: Option<IndexInfo>,
    /// Scan glob pattern (verbose only, for footer).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    /// Wall-clock time for the info operation in milliseconds (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

impl CommandOutput for InfoResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        match &self.index {
            Some(idx) => out.push_str(&format!(
                "{} files, {} fields, {} chunks\n",
                self.files_on_disk,
                self.fields.len(),
                idx.chunks,
            )),
            None => out.push_str(&format!(
                "{} files, {} fields\n",
                self.files_on_disk,
                self.fields.len(),
            )),
        }

        // Metadata table (only when index exists)
        if let Some(idx) = &self.index {
            out.push('\n');
            let mut builder = Builder::default();
            builder.push_record(["model:", &idx.model]);
            if verbose {
                let rev = idx.revision.as_deref().unwrap_or("none");
                builder.push_record(["revision:", rev]);
                builder.push_record(["chunk size:", &idx.chunk_size.to_string()]);
                builder.push_record(["built:", &idx.built_at]);
            }
            builder.push_record(["config:", &idx.config_status]);
            builder.push_record([
                "files:",
                &format!("{}/{}", idx.files_indexed, idx.files_on_disk),
            ]);
            let mut table = builder.build();
            style_compact(&mut table);
            out.push_str(&format!("{table}\n"));
        }

        // Fields table
        if !self.fields.is_empty() {
            out.push('\n');
            if verbose {
                for f in &self.fields {
                    let mut builder = Builder::default();
                    let count_str = match (f.count, f.total_files) {
                        (Some(c), Some(t)) => format!("{c}/{t}"),
                        _ => String::new(),
                    };
                    builder.push_record([
                        format!("\"{}\"", f.name),
                        f.field_type.clone(),
                        count_str,
                    ]);

                    let mut detail_lines = Vec::new();
                    if !f.required.is_empty() {
                        detail_lines.push("  required:".to_string());
                        for g in &f.required {
                            detail_lines.push(format!("    - \"{g}\""));
                        }
                    }
                    detail_lines.push("  allowed:".to_string());
                    for g in &f.allowed {
                        detail_lines.push(format!("    - \"{g}\""));
                    }
                    if !f.hints.is_empty() {
                        detail_lines.push(format!("  hints: {}", format_hints(&f.hints)));
                    }

                    builder.push_record([detail_lines.join("\n"), String::new(), String::new()]);
                    let mut table = builder.build();
                    style_record(&mut table, 3);
                    out.push_str(&format!("{table}\n"));
                }
            } else {
                let mut builder = Builder::default();
                for f in &self.fields {
                    let required_str = if f.required.is_empty() {
                        String::new()
                    } else {
                        let globs: Vec<String> =
                            f.required.iter().map(|g| format!("\"{g}\"")).collect();
                        format!("required: {}", globs.join(", "))
                    };
                    let allowed_str = {
                        let globs: Vec<String> =
                            f.allowed.iter().map(|g| format!("\"{g}\"")).collect();
                        format!("allowed: {}", globs.join(", "))
                    };
                    let mut row = vec![
                        format!("\"{}\"", f.name),
                        f.field_type.clone(),
                        required_str,
                        allowed_str,
                    ];
                    let hints_str = format_hints(&f.hints);
                    if !hints_str.is_empty() {
                        row.push(hints_str);
                    }
                    builder.push_record(row);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
        }

        // Verbose footer
        if verbose {
            if let (Some(glob), Some(ms)) = (&self.glob, self.elapsed_ms) {
                out.push_str(&format!(
                    "\n{} files | glob: \"{glob}\" | {ms}ms\n",
                    self.files_on_disk
                ));
            }
        }

        out
    }
}

/// Read config and index metadata, return a summary of the project state.
#[instrument(name = "info", skip_all)]
pub fn run(path: &Path, verbose: bool) -> anyhow::Result<InfoResult> {
    let start = Instant::now();
    let config = MdvsToml::read(&path.join("mdvs.toml"))?;

    // Scan file count
    let scanned = ScannedFiles::scan(path, &config.scan)?;
    let total_files = scanned.files.len();

    // Count files per field (verbose only)
    let field_counts: HashMap<String, usize> = if verbose {
        let mut counts = HashMap::new();
        for file in &scanned.files {
            if let Some(Value::Object(map)) = &file.data {
                for key in map.keys() {
                    *counts.entry(key.clone()).or_insert(0) += 1;
                }
            }
        }
        counts
    } else {
        HashMap::new()
    };

    // Fields from toml
    let fields: Vec<InfoField> = config
        .fields
        .field
        .iter()
        .map(|f| InfoField {
            name: f.name.clone(),
            field_type: f.field_type.to_string(),
            allowed: f.allowed.clone(),
            required: f.required.clone(),
            count: if verbose {
                Some(*field_counts.get(&f.name).unwrap_or(&0))
            } else {
                None
            },
            total_files: if verbose { Some(total_files) } else { None },
            hints: field_hints(&f.name),
        })
        .collect();

    // Index info (if index exists)
    let backend = Backend::parquet(path, config.internal_prefix());
    let index = if backend.exists() {
        let build_meta = backend.read_metadata()?;
        let idx_stats = backend.stats()?;
        match (build_meta, idx_stats) {
            (Some(meta), Some(stats)) => {
                let config_match = config.embedding_model.as_ref() == Some(&meta.embedding_model)
                    && config.chunking.as_ref() == Some(&meta.chunking);
                Some(IndexInfo {
                    model: meta.embedding_model.name,
                    revision: meta.embedding_model.revision,
                    chunk_size: meta.chunking.max_chunk_size,
                    files_indexed: stats.files_indexed,
                    files_on_disk: total_files,
                    chunks: stats.chunks,
                    built_at: meta.built_at,
                    config_status: if config_match {
                        "match".to_string()
                    } else {
                        "changed — rebuild recommended".to_string()
                    },
                })
            }
            _ => None,
        }
    } else {
        None
    };

    Ok(InfoResult {
        scan_glob: config.scan.glob.clone(),
        files_on_disk: total_files,
        fields,
        ignored_fields: config.fields.ignore.clone(),
        index,
        glob: if verbose {
            Some(config.scan.glob.clone())
        } else {
            None
        },
        elapsed_ms: if verbose {
            Some(start.elapsed().as_millis() as u64)
        } else {
            None
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
    use std::fs;

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Rust Programming\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Rust Programming\nBody.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: Cooking Recipes\ndraft: true\n---\n# Cooking Recipes\nBody.",
        )
        .unwrap();
    }

    fn write_config(dir: &Path) {
        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: true },
            fields: FieldsConfig {
                ignore: vec!["internal_id".into()],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                    },
                ],
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    async fn init_and_build(dir: &Path) {
        crate::cmd::init::run(
            dir,
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
            false, // verbose
        )
        .await
        .unwrap();
    }

    #[test]
    fn info_no_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        write_config(tmp.path());

        let result = run(tmp.path(), false).unwrap();

        assert_eq!(result.scan_glob, "**");
        assert_eq!(result.files_on_disk, 2);
        assert_eq!(result.fields.len(), 3);
        assert_eq!(result.fields[0].name, "title");
        assert_eq!(result.fields[0].field_type, "String");
        assert_eq!(result.fields[1].name, "tags");
        assert_eq!(result.fields[1].field_type, "String[]");
        assert_eq!(result.fields[2].name, "draft");
        assert_eq!(result.fields[2].field_type, "Boolean");
        assert_eq!(result.ignored_fields, vec!["internal_id"]);
        assert!(result.index.is_none());
    }

    #[tokio::test]
    async fn info_with_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let result = run(tmp.path(), false).unwrap();

        assert_eq!(result.files_on_disk, 2);
        assert!(result.index.is_some());
        let idx = result.index.unwrap();
        assert_eq!(idx.model, "minishlab/potion-base-8M");
        assert_eq!(idx.files_indexed, 2);
        assert!(idx.chunks > 0);
        assert_eq!(idx.config_status, "match");
    }

    #[tokio::test]
    async fn info_config_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // Change chunk_size in toml
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 512;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), false).unwrap();

        assert!(result.index.is_some());
        let idx = result.index.unwrap();
        assert_eq!(idx.config_status, "changed — rebuild recommended");
    }

    #[test]
    fn info_hints_for_special_char_field_names() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("note.md"),
            "---\nauthor's_note: hello\n---\n# Note\nBody.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: true },
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![crate::schema::config::TomlField {
                    name: "author's_note".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                }],
            },
            embedding_model: None,
            chunking: None,
            search: None,
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), false).unwrap();

        assert_eq!(result.fields.len(), 1);
        assert_eq!(result.fields[0].name, "author's_note");
        assert!(result.fields[0]
            .hints
            .contains(&FieldHint::EscapeSingleQuotes));
    }
}
