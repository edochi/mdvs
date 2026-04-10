use crate::discover::scan::ScannedFiles;
use crate::index::backend::Backend;
use crate::outcome::commands::InfoOutcome;
use crate::outcome::{Outcome, ReadConfigOutcome, ReadIndexOutcome, ScanOutcome};
use crate::output::{FieldHint, field_hints};
use crate::schema::config::MdvsToml;
use crate::step::{CommandResult, ErrorKind, StepEntry};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

/// A single field definition for info display.
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
    /// Whether null values are accepted for this field.
    pub nullable: bool,
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

/// Built index metadata for info display.
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

/// Read config, scan files, and read index metadata.
#[instrument(name = "info", skip_all)]
pub fn run(path: &Path, _verbose: bool) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // 1. Read config — calls MdvsToml::read() + validate() directly
    let config_start = Instant::now();
    let config_path_buf = path.join("mdvs.toml");
    let config = match MdvsToml::read(&config_path_buf) {
        Ok(cfg) => match cfg.validate() {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::ReadConfig(ReadConfigOutcome {
                        config_path: config_path_buf.display().to_string(),
                    }),
                    config_start.elapsed().as_millis() as u64,
                ));
                Some(cfg)
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::User,
                    format!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'"),
                    config_start.elapsed().as_millis() as u64,
                ));
                None
            }
        },
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                e.to_string(),
                config_start.elapsed().as_millis() as u64,
            ));
            None
        }
    };

    let config = match config {
        Some(c) => c,
        None => {
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    // 2. Scan — calls ScannedFiles::scan() directly
    let scan_start = Instant::now();
    let scanned = match ScannedFiles::scan(path, &config.scan) {
        Ok(s) => {
            steps.push(StepEntry::ok(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: config.scan.glob.clone(),
                }),
                scan_start.elapsed().as_millis() as u64,
            ));
            Some(s)
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                scan_start.elapsed().as_millis() as u64,
            ));
            None
        }
    };

    // 3. Read index — calls Backend methods directly
    let index_start = Instant::now();
    let backend = Backend::parquet(path);
    let index_data = if !backend.exists() {
        steps.push(StepEntry::ok(
            Outcome::ReadIndex(ReadIndexOutcome {
                exists: false,
                files_indexed: 0,
                chunks: 0,
            }),
            index_start.elapsed().as_millis() as u64,
        ));
        None
    } else {
        let build_meta = backend.read_metadata().ok().flatten();
        let idx_stats = backend.stats().ok().flatten();
        match (build_meta, idx_stats) {
            (Some(metadata), Some(stats)) => {
                steps.push(StepEntry::ok(
                    Outcome::ReadIndex(ReadIndexOutcome {
                        exists: true,
                        files_indexed: stats.files_indexed,
                        chunks: stats.chunks,
                    }),
                    index_start.elapsed().as_millis() as u64,
                ));
                Some((metadata, stats))
            }
            _ => {
                steps.push(StepEntry::ok(
                    Outcome::ReadIndex(ReadIndexOutcome {
                        exists: false,
                        files_indexed: 0,
                        chunks: 0,
                    }),
                    index_start.elapsed().as_millis() as u64,
                ));
                None
            }
        }
    };

    // Build InfoOutcome from config + scanned + index_data
    let empty_files = Vec::new();
    let files = scanned.as_ref().map(|s| &s.files).unwrap_or(&empty_files);
    let total_files = files.len();

    let field_counts: HashMap<String, usize> = {
        let mut counts = HashMap::new();
        for file in files {
            if let Some(Value::Object(map)) = &file.data {
                for key in map.keys() {
                    *counts.entry(key.clone()).or_insert(0) += 1;
                }
            }
        }
        counts
    };

    let fields: Vec<InfoField> = config
        .fields
        .field
        .iter()
        .map(|f| InfoField {
            name: f.name.clone(),
            field_type: f.field_type.to_string(),
            allowed: f.allowed.clone(),
            required: f.required.clone(),
            nullable: f.nullable,
            count: Some(*field_counts.get(&f.name).unwrap_or(&0)),
            total_files: Some(total_files),
            hints: field_hints(&f.name),
        })
        .collect();

    let index = index_data.map(|(metadata, stats)| {
        let config_match = config.embedding_model.as_ref() == Some(&metadata.embedding_model)
            && config.chunking.as_ref() == Some(&metadata.chunking);
        IndexInfo {
            model: metadata.embedding_model.name,
            revision: metadata.embedding_model.revision,
            chunk_size: metadata.chunking.max_chunk_size,
            files_indexed: stats.files_indexed,
            files_on_disk: total_files,
            chunks: stats.chunks,
            built_at: metadata.built_at,
            config_status: if config_match {
                "match".to_string()
            } else {
                "changed — rebuild recommended".to_string()
            },
        }
    });

    CommandResult {
        steps,
        result: Ok(Outcome::Info(Box::new(InfoOutcome {
            scan_glob: config.scan.glob.clone(),
            files_on_disk: total_files,
            fields,
            ignored_fields: config.fields.ignore.clone(),
            index,
        }))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use crate::schema::config::{FieldsConfig, MdvsToml, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
    use crate::step::CommandResult;
    use std::fs;

    fn unwrap_info(result: &CommandResult) -> &InfoOutcome {
        match &result.result {
            Ok(Outcome::Info(o)) => o,
            other => panic!("expected Ok(Info), got: {other:?}"),
        }
    }

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
        let mut config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec!["internal_id".into()],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                        nullable: false,
                        constraints: None,
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                    },
                ],
                max_categories: 10,
                min_category_repetition: 2,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: std::collections::HashMap::new(),
            }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    async fn init_and_build(dir: &Path) {
        let step = crate::cmd::init::run(dir, "**", false, false, true, false, false);
        assert!(!crate::step::has_failed(&step));
        let output = crate::cmd::build::run(dir, None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));
    }

    #[test]
    fn info_no_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        write_config(tmp.path());
        let step = run(tmp.path(), false);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_info(&step);
        assert_eq!(result.scan_glob, "**");
        assert_eq!(result.files_on_disk, 2);
        assert_eq!(result.fields.len(), 3);
        assert_eq!(result.fields[0].name, "draft"); // alphabetically sorted
        assert_eq!(result.ignored_fields, vec!["internal_id"]);
        assert!(result.index.is_none());
    }

    #[tokio::test]
    async fn info_with_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;
        let step = run(tmp.path(), false);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_info(&step);
        assert_eq!(result.files_on_disk, 2);
        assert!(result.index.is_some());
        let idx = result.index.as_ref().unwrap();
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
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 512;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();
        let step = run(tmp.path(), false);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_info(&step);
        assert!(result.index.is_some());
        assert_eq!(
            result.index.as_ref().unwrap().config_status,
            "changed — rebuild recommended"
        );
    }

    #[test]
    fn info_hints_for_special_char_field_names() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("note.md"),
            "---\nauthor's_note: hello\n---\n# Note\nBody.",
        )
        .unwrap();
        let mut config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![crate::schema::config::TomlField {
                    name: "author's_note".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                }],
                max_categories: 10,
                min_category_repetition: 2,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();
        let step = run(tmp.path(), false);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_info(&step);
        assert_eq!(result.fields.len(), 1);
        assert_eq!(result.fields[0].name, "author's_note");
        assert!(
            result.fields[0]
                .hints
                .contains(&FieldHint::EscapeSingleQuotes)
        );
    }
}
