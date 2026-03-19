use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{content_hash, BuildMetadata, FileRow};
use crate::outcome::commands::BuildOutcome;
use crate::outcome::{
    ClassifyOutcome, EmbedFilesOutcome, LoadModelOutcome, Outcome, ReadConfigOutcome, ScanOutcome,
    ValidateOutcome, WriteIndexOutcome,
};
use crate::pipeline::classify::run_classify;
use crate::pipeline::embed::run_embed_files;
use crate::pipeline::load_model::run_load_model;
use crate::pipeline::read_config::run_read_config;
use crate::pipeline::scan::run_scan;
use crate::pipeline::validate::run_validate;
use crate::pipeline::write_index::run_write_index;
use crate::pipeline::ProcessingStepResult;
use crate::schema::config::{BuildConfig, MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use crate::step::{from_pipeline_result, ErrorKind, Step, StepError, StepOutcome};
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

// ============================================================================
// run()
// ============================================================================

/// Validate frontmatter, chunk, embed, and write Parquet files to `.mdvs/`.
#[instrument(name = "build", skip_all)]
pub async fn run(
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
    no_update: bool,
    _verbose: bool,
) -> Step<Outcome> {
    let start = Instant::now();
    let mut substeps = Vec::new();

    // 1. Read config
    let (read_config_result, config) = run_read_config(path);
    substeps.push(from_pipeline_result(read_config_result, |o| {
        Outcome::ReadConfig(ReadConfigOutcome {
            config_path: o.config_path.clone(),
        })
    }));
    let config = match config {
        Some(c) => c,
        None => return fail_from_last(&mut substeps, start, 7),
    };

    // 2. Auto-update (conditional)
    let should_update = !no_update && config.build.as_ref().is_some_and(|b| b.auto_update);
    if should_update {
        let update_step = crate::cmd::update::run(path, &[], false, false, false).await;
        if update_step.has_failed_step() {
            substeps.push(update_step);
            return fail_msg(
                &mut substeps,
                start,
                ErrorKind::User,
                "auto-update failed",
                6,
            );
        }
        substeps.push(update_step);
    }

    // Re-read config if auto-update ran
    let mut config = if should_update {
        let (re_read, cfg) = run_read_config(path);
        substeps.push(from_pipeline_result(re_read, |o| {
            Outcome::ReadConfig(ReadConfigOutcome {
                config_path: o.config_path.clone(),
            })
        }));
        match cfg {
            Some(c) => c,
            None => return fail_from_last(&mut substeps, start, 6),
        }
    } else {
        config
    };

    // Mutate config (inline, not a step)
    let mutation_error = mutate_config(
        &mut config,
        path,
        set_model,
        set_revision,
        set_chunk_size,
        force,
    );

    // 3. Scan (mutation errors land here)
    if let Some(msg) = mutation_error {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: msg,
                }),
                elapsed_ms: 0,
            },
        });
        return fail_from_last(&mut substeps, start, 5);
    }

    let (scan_result, scanned) = run_scan(path, &config.scan);
    substeps.push(from_pipeline_result(scan_result, |o| {
        Outcome::Scan(ScanOutcome {
            files_found: o.files_found,
            glob: o.glob.clone(),
        })
    }));
    let scanned = match scanned {
        Some(s) => s,
        None => return fail_from_last(&mut substeps, start, 5),
    };

    // 4. Validate
    if scanned.files.is_empty() {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: format!("no markdown files found in '{}'", path.display()),
                }),
                elapsed_ms: 0,
            },
        });
        return fail_from_last(&mut substeps, start, 4);
    }

    let (validate_result, validation_data) = run_validate(&scanned, &config, false);
    let (violations, new_fields) = match validation_data {
        Some((v, nf)) => (v, nf),
        None => {
            substeps.push(from_pipeline_result(validate_result, |o| {
                Outcome::Validate(ValidateOutcome {
                    files_checked: o.files_checked,
                    violations: vec![],
                    new_fields: vec![],
                })
            }));
            return fail_from_last(&mut substeps, start, 4);
        }
    };

    // Build validate substep with actual violation/new_fields data
    let validate_step = Step {
        substeps: vec![],
        outcome: match validate_result {
            ProcessingStepResult::Completed(step) => StepOutcome::Complete {
                result: Ok(Outcome::Validate(ValidateOutcome {
                    files_checked: step.output.files_checked,
                    violations: violations.clone(),
                    new_fields: new_fields.clone(),
                })),
                elapsed_ms: step.elapsed_ms,
            },
            ProcessingStepResult::Failed(err) => StepOutcome::Complete {
                result: Err(StepError {
                    kind: crate::step::convert_error_kind(err.kind),
                    message: err.message,
                }),
                elapsed_ms: 0,
            },
            ProcessingStepResult::Skipped => StepOutcome::Skipped,
        },
    };
    substeps.push(validate_step);

    // Abort on violations
    if !violations.is_empty() {
        for _ in 0..4 {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            });
        }
        return Step {
            substeps,
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: format!(
                        "{} violation(s) found. Run `mdvs check` for details.",
                        violations.len()
                    ),
                }),
                elapsed_ms: start.elapsed().as_millis() as u64,
            },
        };
    }

    // Pre-checks for classify
    let schema_fields: Vec<(String, FieldType)> = match config
        .fields
        .field
        .iter()
        .map(|f| {
            let ft = FieldType::try_from(&f.field_type)
                .map_err(|e| format!("invalid field type for '{}': {}", f.name, e))?;
            Ok((f.name.clone(), ft))
        })
        .collect::<Result<Vec<_>, String>>()
    {
        Ok(sf) => sf,
        Err(msg) => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::Application,
                        message: msg,
                    }),
                    elapsed_ms: 0,
                },
            });
            return fail_from_last(&mut substeps, start, 3);
        }
    };

    let embedding = match config.embedding_model.as_ref() {
        Some(e) => e,
        None => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::User,
                        message: "missing [embedding_model] in mdvs.toml".into(),
                    }),
                    elapsed_ms: 0,
                },
            });
            return fail_from_last(&mut substeps, start, 3);
        }
    };
    let chunking = match config.chunking.as_ref() {
        Some(c) => c,
        None => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::User,
                        message: "missing [chunking] in mdvs.toml".into(),
                    }),
                    elapsed_ms: 0,
                },
            });
            return fail_from_last(&mut substeps, start, 3);
        }
    };
    let backend = Backend::parquet(path);
    let config_change_error = detect_config_changes(&backend, embedding, chunking, &config, force);

    // 5. Classify
    let full_rebuild = force || !backend.exists();

    if let Some(msg) = config_change_error {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: msg,
                }),
                elapsed_ms: 0,
            },
        });
        return fail_from_last(&mut substeps, start, 3);
    }

    let existing_index = if full_rebuild {
        vec![]
    } else {
        match backend.read_file_index() {
            Ok(idx) => idx,
            Err(e) => {
                substeps.push(Step {
                    substeps: vec![],
                    outcome: StepOutcome::Complete {
                        result: Err(StepError {
                            kind: ErrorKind::Application,
                            message: e.to_string(),
                        }),
                        elapsed_ms: 0,
                    },
                });
                return fail_from_last(&mut substeps, start, 3);
            }
        }
    };
    let existing_chunks = if full_rebuild {
        vec![]
    } else {
        match backend.read_chunk_rows() {
            Ok(crs) => crs,
            Err(e) => {
                substeps.push(Step {
                    substeps: vec![],
                    outcome: StepOutcome::Complete {
                        result: Err(StepError {
                            kind: ErrorKind::Application,
                            message: e.to_string(),
                        }),
                        elapsed_ms: 0,
                    },
                });
                return fail_from_last(&mut substeps, start, 3);
            }
        }
    };

    let (classify_result, classify_data) =
        run_classify(&scanned, &existing_index, existing_chunks, full_rebuild);
    substeps.push(from_pipeline_result(classify_result, |o| {
        Outcome::Classify(ClassifyOutcome {
            full_rebuild: o.full_rebuild,
            needs_embedding: o.needs_embedding,
            unchanged: o.unchanged,
            removed: o.removed,
        })
    }));
    let classify_data = match classify_data {
        Some(d) => d,
        None => return fail_from_last(&mut substeps, start, 3),
    };

    let needs_embedding = !classify_data.needs_embedding.is_empty();

    // 6. Load model
    let (load_model_result, embedder) = if needs_embedding {
        run_load_model(embedding)
    } else {
        (ProcessingStepResult::Skipped, None)
    };
    substeps.push(from_pipeline_result(load_model_result, |o| {
        Outcome::LoadModel(LoadModelOutcome {
            model_name: o.model_name.clone(),
            dimension: o.dimension,
        })
    }));

    // Dimension check
    let dim_error = if full_rebuild {
        None
    } else {
        match &embedder {
            Some(emb) => match backend.embedding_dimension() {
                Ok(Some(existing_dim)) => {
                    let model_dim = emb.dimension() as i32;
                    if existing_dim != model_dim {
                        Some(format!("dimension mismatch: model produces {model_dim}-dim embeddings but existing index has {existing_dim}-dim"))
                    } else {
                        None
                    }
                }
                Ok(None) => None,
                Err(e) => Some(e.to_string()),
            },
            None if needs_embedding => None,
            None => None,
        }
    };

    if needs_embedding && embedder.is_none() {
        for _ in 0..2 {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            });
        }
        return fail_msg(
            &mut substeps,
            start,
            ErrorKind::Application,
            "model loading failed",
            0,
        );
    }

    // 7. Embed files
    let max_chunk_size = chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    if let Some(msg) = dim_error {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: msg,
                }),
                elapsed_ms: 0,
            },
        });
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        }); // write_index
        return fail_from_last_skip(&mut substeps, start, 0);
    }

    let (embed_result, embed_data) = if needs_embedding {
        let emb = embedder.as_ref().unwrap();
        run_embed_files(&classify_data.needs_embedding, emb, max_chunk_size).await
    } else {
        (ProcessingStepResult::Skipped, None)
    };
    substeps.push(from_pipeline_result(embed_result, |o| {
        Outcome::EmbedFiles(EmbedFilesOutcome {
            files_embedded: o.files_embedded,
            chunks_produced: o.chunks_produced,
        })
    }));

    if needs_embedding
        && embed_data.is_none()
        && !matches!(substeps.last().unwrap().outcome, StepOutcome::Skipped)
    {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        }); // write_index
        return fail_from_last_skip(&mut substeps, start, 0);
    }

    // 8. Write index
    let file_rows: Vec<FileRow> = scanned
        .files
        .iter()
        .map(|f| {
            let filename = f.path.display().to_string();
            let file_id = classify_data.file_id_map[&filename].clone();
            FileRow {
                file_id,
                filename,
                frontmatter: f.data.clone(),
                content_hash: content_hash(&f.content),
                built_at,
            }
        })
        .collect();

    let mut chunk_rows = classify_data.retained_chunks;
    let mut embedded_details = Vec::new();
    if let Some(ed) = embed_data {
        chunk_rows.extend(ed.chunk_rows);
        embedded_details = ed.details;
    }

    let build_meta = BuildMetadata {
        embedding_model: embedding.clone(),
        chunking: chunking.clone(),
        glob: config.scan.glob.clone(),
        built_at: chrono::Utc::now().to_rfc3339(),
    };

    let write_index_result = run_write_index(
        &backend,
        &schema_fields,
        &file_rows,
        &chunk_rows,
        build_meta,
    );
    substeps.push(from_pipeline_result(write_index_result, |o| {
        Outcome::WriteIndex(WriteIndexOutcome {
            files_written: o.files_written,
            chunks_written: o.chunks_written,
        })
    }));

    if crate::step::has_failed(substeps.last().unwrap()) {
        return fail_from_last_skip(&mut substeps, start, 0);
    }

    // Assemble BuildOutcome
    let chunks_embedded: usize = embedded_details.iter().map(|d| d.chunks).sum();
    let chunks_total = chunk_rows.len();
    let chunks_unchanged = chunks_total - chunks_embedded;

    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Build(Box::new(BuildOutcome {
                full_rebuild: classify_data.full_rebuild,
                files_total: file_rows.len(),
                files_embedded: classify_data.needs_embedding.len(),
                files_unchanged: file_rows.len() - classify_data.needs_embedding.len(),
                files_removed: classify_data.removed_count,
                chunks_total,
                chunks_embedded,
                chunks_unchanged,
                chunks_removed: classify_data.chunks_removed,
                new_fields,
                embedded_files: embedded_details,
                removed_files: classify_data.removed_details,
            }))),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Push N Skipped substeps, extract error from last substep, return failed command Step.
fn fail_from_last(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    skipped: usize,
) -> Step<Outcome> {
    let msg = match substeps.last().map(|s| &s.outcome) {
        Some(StepOutcome::Complete { result: Err(e), .. }) => e.message.clone(),
        _ => "step failed".into(),
    };
    for _ in 0..skipped {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    }
    Step {
        substeps: std::mem::take(substeps),
        outcome: StepOutcome::Complete {
            result: Err(StepError {
                kind: ErrorKind::Application,
                message: msg,
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Push N Skipped substeps, return failed command Step with a specific message.
fn fail_msg(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    kind: ErrorKind,
    msg: &str,
    skipped: usize,
) -> Step<Outcome> {
    for _ in 0..skipped {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    }
    Step {
        substeps: std::mem::take(substeps),
        outcome: StepOutcome::Complete {
            result: Err(StepError {
                kind,
                message: msg.into(),
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Return failed command Step (no additional Skipped substeps).
fn fail_from_last_skip(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    _skipped: usize,
) -> Step<Outcome> {
    let msg = match substeps.iter().rev().find_map(|s| match &s.outcome {
        StepOutcome::Complete { result: Err(e), .. } => Some(e.message.clone()),
        _ => None,
    }) {
        Some(m) => m,
        None => "step failed".into(),
    };
    Step {
        substeps: std::mem::take(substeps),
        outcome: StepOutcome::Complete {
            result: Err(StepError {
                kind: ErrorKind::Application,
                message: msg,
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Normalize a revision string: empty and "None" are treated as unset.
fn normalize_revision(s: &str) -> Option<String> {
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(s.to_string())
    }
}

/// Apply config mutations: fill missing build sections, apply --set-* flags.
/// Returns `Some(error_message)` if a flag requires --force but wasn't given.
fn mutate_config(
    config: &mut MdvsToml,
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
) -> Option<String> {
    let config_path = path.join("mdvs.toml");
    let mut config_changed = false;

    match config.embedding_model {
        None => {
            config.embedding_model = Some(EmbeddingModelConfig {
                provider: "model2vec".to_string(),
                name: set_model.unwrap_or(DEFAULT_MODEL).to_string(),
                revision: set_revision.and_then(normalize_revision),
            });
            config_changed = true;
        }
        Some(ref mut em) if set_model.is_some() || set_revision.is_some() => {
            if !force {
                return Some(
                    "--set-model/--set-revision require --force (changes model, triggers full re-embed)"
                        .to_string(),
                );
            }
            if let Some(m) = set_model {
                em.name = m.to_string();
            }
            if let Some(r) = set_revision {
                em.revision = normalize_revision(r);
            }
            config_changed = true;
        }
        Some(_) => {}
    }

    match config.chunking {
        None => {
            config.chunking = Some(ChunkingConfig {
                max_chunk_size: set_chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
            });
            config_changed = true;
        }
        Some(ref mut ch) if set_chunk_size.is_some() => {
            if !force {
                return Some(
                    "--set-chunk-size requires --force (changes chunking, triggers full re-embed)"
                        .to_string(),
                );
            }
            // Safe: match guard ensures set_chunk_size.is_some()
            ch.max_chunk_size = set_chunk_size.expect("guarded by is_some()");
            config_changed = true;
        }
        Some(_) => {}
    }

    if config.search.is_none() {
        config.search = Some(SearchConfig {
            default_limit: 10,
            auto_update: true,
            auto_build: true,
            internal_prefix: String::new(),
            aliases: std::collections::HashMap::new(),
        });
        config_changed = true;
    }

    if config.build.is_none() {
        config.build = Some(BuildConfig { auto_update: true });
        config_changed = true;
    }

    if config_changed {
        if let Err(e) = config.write(&config_path) {
            return Some(format!("failed to write config: {e}"));
        }
    }

    None
}

/// Detect manual config changes against the existing parquet metadata.
/// Returns `Some(error_message)` if config changed and --force not given.
pub(crate) fn detect_config_changes(
    backend: &Backend,
    embedding: &EmbeddingModelConfig,
    chunking: &ChunkingConfig,
    _config: &MdvsToml,
    force: bool,
) -> Option<String> {
    if force {
        return None;
    }
    let meta = match backend.read_metadata() {
        Ok(Some(m)) => m,
        Ok(None) => return None, // first build, no metadata
        Err(e) => return Some(e.to_string()),
    };

    let mut mismatches = Vec::new();
    if meta.embedding_model != *embedding {
        mismatches.push(format!(
            "model: '{}' (rev {:?}) -> '{}' (rev {:?})",
            meta.embedding_model.name,
            meta.embedding_model.revision,
            embedding.name,
            embedding.revision,
        ));
    }
    if meta.chunking != *chunking {
        mismatches.push(format!(
            "chunk_size: {} -> {}",
            meta.chunking.max_chunk_size, chunking.max_chunk_size,
        ));
    }

    if mismatches.is_empty() {
        None
    } else {
        Some(format!(
            "config changed since last build:\n  {}\nUse --force to rebuild with new config",
            mismatches.join("\n  "),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::storage::{
        read_build_metadata, read_chunk_rows, read_file_index, read_parquet,
    };
    use crate::schema::config::MdvsToml;
    use datafusion::arrow::datatypes::DataType;
    use std::collections::{HashMap, HashSet};
    use std::fs;

    fn unwrap_build(step: &Step<Outcome>) -> &BuildOutcome {
        match &step.outcome {
            StepOutcome::Complete {
                result: Ok(Outcome::Build(o)),
                ..
            } => o,
            other => panic!("expected Ok(Build), got: {other:?}"),
        }
    }

    fn unwrap_error(step: &Step<Outcome>) -> &StepError {
        match &step.outcome {
            StepOutcome::Complete { result: Err(e), .. } => e,
            other => panic!("expected Err, got: {other:?}"),
        }
    }

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text about the world.",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(output.has_failed_step());
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only, no build)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,  // ignore bare files
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!output.has_failed_step());

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            !output.has_failed_step(),
            "first build failed: {:#?}",
            output
        );

        // Run build again (tests standalone rebuild)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step(), "build failed: {:#?}", output);
        assert!(!output.has_failed_step());

        // Verify Parquet files exist
        let files_path = tmp.path().join(".mdvs/files.parquet");
        let chunks_path = tmp.path().join(".mdvs/chunks.parquet");
        assert!(files_path.exists());
        assert!(chunks_path.exists());

        // Verify files.parquet row count
        let file_batches = read_parquet(&files_path).unwrap();
        let file_rows: usize = file_batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(file_rows, 2); // 2 files with frontmatter

        // Verify chunks.parquet has embeddings with correct dimension
        let chunk_batches = read_parquet(&chunks_path).unwrap();
        let chunk_rows: usize = chunk_batches.iter().map(|b| b.num_rows()).sum();
        assert!(chunk_rows > 0);

        let embedding_field = chunk_batches[0]
            .schema()
            .field_with_name("embedding")
            .unwrap()
            .clone();
        if let DataType::FixedSizeList(_, dim) = embedding_field.data_type() {
            assert!(*dim > 0);
        } else {
            panic!("expected FixedSizeList for embedding column");
        }

        // Verify build metadata on files.parquet
        let meta = read_build_metadata(&files_path).unwrap();
        assert!(meta.is_some(), "build metadata should be present");
        let meta = meta.unwrap();
        assert_eq!(meta.embedding_model.name, "minishlab/potion-base-8M");
        assert_eq!(meta.chunking.max_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(meta.glob, "**");
    }

    #[tokio::test]
    async fn dimension_mismatch() {
        use crate::index::storage::{build_chunks_batch, write_parquet, ChunkRow};

        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!output.has_failed_step());

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        // Overwrite chunks.parquet with wrong dimension (2 instead of actual)
        let bad_chunks = vec![ChunkRow {
            chunk_id: "bad".into(),
            file_id: "bad".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![0.1, 0.2], // dim=2
        }];
        let bad_batch = build_chunks_batch(&bad_chunks, 2);
        write_parquet(&tmp.path().join(".mdvs/chunks.parquet"), &bad_batch).unwrap();

        // Add a new file so model gets loaded (incremental detects new file)
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\ntags:\n  - test\ndraft: true\n---\n# New\nNew content.",
        )
        .unwrap();

        // Build should fail with dimension mismatch when model loads
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("dimension mismatch"));
    }

    #[tokio::test]
    async fn dimension_mismatch_with_force_succeeds() {
        use crate::index::storage::{build_chunks_batch, write_parquet, ChunkRow};

        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!output.has_failed_step());

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        // Overwrite chunks.parquet with wrong dimension (2 instead of actual)
        let bad_chunks = vec![ChunkRow {
            chunk_id: "bad".into(),
            file_id: "bad".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![0.1, 0.2], // dim=2
        }];
        let bad_batch = build_chunks_batch(&bad_chunks, 2);
        write_parquet(&tmp.path().join(".mdvs/chunks.parquet"), &bad_batch).unwrap();

        // Build with --force should succeed despite dimension mismatch
        let output = run(tmp.path(), None, None, None, true, true, false).await;
        assert!(
            !output.has_failed_step(),
            "expected success with --force, got failed step"
        );
        assert!(!output.has_failed_step());
        let result = unwrap_build(&output);
        assert!(result.full_rebuild);
    }

    #[tokio::test]
    async fn missing_build_sections_filled() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init (schema only, no build sections in toml)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!output.has_failed_step());

        // Verify no model/chunking sections (auto-flag sections are present from init)
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());

        // Build should fill defaults and succeed
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step(), "build failed: {:#?}", output);

        // Verify sections were written
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.embedding_model.as_ref().unwrap().name, DEFAULT_MODEL);
        assert!(config.embedding_model.as_ref().unwrap().revision.is_none());
        assert_eq!(
            config.chunking.as_ref().unwrap().max_chunk_size,
            DEFAULT_CHUNK_SIZE
        );
        assert_eq!(config.search.as_ref().unwrap().default_limit, 10);

        // Verify index was created
        assert!(tmp.path().join(".mdvs/files.parquet").exists());
        assert!(tmp.path().join(".mdvs/chunks.parquet").exists());
    }

    #[tokio::test]
    async fn set_model_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!output.has_failed_step());

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        // Try to change model without --force
        let output = run(
            tmp.path(),
            Some("other-model"),
            None,
            None,
            false,
            true,
            false,
        )
        .await;
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("--force"));
    }

    #[tokio::test]
    async fn set_chunk_size_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let output = run(tmp.path(), None, None, Some(512), false, true, false).await;
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("--force"));
    }

    #[tokio::test]
    async fn set_model_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        // Change chunk size with --force (same model so no dimension mismatch)
        let output = run(tmp.path(), None, None, Some(512), true, true, false).await;
        assert!(
            !output.has_failed_step(),
            "build with --force failed: {:#?}",
            output
        );

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.chunking.as_ref().unwrap().max_chunk_size, 512);
    }

    #[tokio::test]
    async fn manual_config_change_detected() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections + parquets)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("config changed since last build"));
        assert!(err.message.contains("chunk_size"));

        // Build with --force should succeed
        let output = run(tmp.path(), None, None, None, true, true, false).await;
        assert!(
            !output.has_failed_step(),
            "build with --force failed: {:#?}",
            output
        );
    }

    #[tokio::test]
    async fn build_aborts_on_wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // draft is string "yes" in the file but declared as Boolean in toml
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Hello\nBody.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        // Declare as Boolean, but file has String → WrongType violation
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
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
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            crate::step::has_violations(&output),
            "expected validation violations"
        );
    }

    #[tokio::test]
    async fn build_aborts_on_missing_required() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // post1 has tags, post2 does not — tags required in blog/**
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n---\n# Hello\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\n---\n# World\nBody.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Array {
                            array: Box::new(crate::schema::shared::FieldTypeSerde::Scalar(
                                "String".into(),
                            )),
                        },
                        allowed: vec!["**".into()],
                        required: vec!["blog/**".into()],
                        nullable: false,
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
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            crate::step::has_violations(&output),
            "expected validation violations"
        );
    }

    #[tokio::test]
    async fn build_succeeds_with_new_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // File has title + author, but toml only declares title
        // author is a "new field" — informational, should not block build
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\nauthor: Alice\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![crate::schema::config::TomlField {
                    name: "title".into(),
                    field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                }],
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
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build should succeed despite unknown "author" field
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            !output.has_failed_step(),
            "build should succeed with new fields: {:#?}",
            output
        );

        // Verify index was created
        assert!(tmp.path().join(".mdvs/files.parquet").exists());
        assert!(tmp.path().join(".mdvs/chunks.parquet").exists());
    }

    // ========================================================================
    // Incremental build integration tests
    // ========================================================================

    /// Read file_id→filename map and chunk_id→file_id map from existing parquets.
    fn read_index_state(dir: &Path) -> (HashMap<String, String>, Vec<(String, String)>) {
        let file_index = read_file_index(&dir.join(".mdvs/files.parquet")).unwrap();
        let file_map: HashMap<String, String> = file_index
            .iter()
            .map(|e| (e.filename.clone(), e.file_id.clone()))
            .collect();
        let chunks = read_chunk_rows(&dir.join(".mdvs/chunks.parquet")).unwrap();
        let chunk_pairs: Vec<(String, String)> = chunks
            .iter()
            .map(|c| (c.chunk_id.clone(), c.file_id.clone()))
            .collect();
        (file_map, chunk_pairs)
    }

    #[tokio::test]
    async fn incremental_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());

        // Build again with no changes
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // chunk_ids preserved (same chunks carried forward)
        let old_chunk_ids: HashSet<&str> =
            chunks_before.iter().map(|(id, _)| id.as_str()).collect();
        let new_chunk_ids: HashSet<&str> = chunks_after.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn incremental_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());
        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: Third\ntags:\n  - new\ndraft: false\n---\n# Third\nNew post content.",
        )
        .unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // Old file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // New file added
        assert!(files_after.contains_key("blog/post3.md"));
        assert_eq!(files_after.len(), 3);

        // Old chunks preserved, new chunks added
        for (chunk_id, _) in &chunks_before {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "old chunk {chunk_id} missing",
            );
        }
        assert!(chunks_after.len() > chunks_before.len());
    }

    #[tokio::test]
    async fn incremental_edited_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let post1_id = files_before["blog/post1.md"].clone();
        let post2_id = files_before["blog/post2.md"].clone();

        // Chunks belonging to each file
        let post1_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        let post2_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post2_id)
            .map(|(cid, _)| cid.clone())
            .collect();

        // Edit post1's body (keep same frontmatter)
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nCompletely different body text.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // file_ids preserved for both files
        assert_eq!(files_after["blog/post1.md"], post1_id);
        assert_eq!(files_after["blog/post2.md"], post2_id);

        // post2 chunks preserved (unchanged file)
        for chunk_id in &post2_chunks {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "post2 chunk {chunk_id} should be preserved",
            );
        }
        // post1 chunks replaced (edited file — new chunk_ids)
        let new_post1_chunks: HashSet<String> = chunks_after
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        assert!(!new_post1_chunks.is_empty());
        for old_id in &post1_chunks {
            assert!(
                !new_post1_chunks.contains(old_id),
                "old chunk should be replaced"
            );
        }
    }

    #[tokio::test]
    async fn incremental_removed_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_before, _) = read_index_state(tmp.path());
        assert_eq!(files_before.len(), 2);

        // Remove post2
        fs::remove_file(tmp.path().join("blog/post2.md")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_after, chunks_after) = read_index_state(tmp.path());

        assert_eq!(files_after.len(), 1);
        assert!(files_after.contains_key("blog/post1.md"));
        assert!(!files_after.contains_key("blog/post2.md"));

        // No chunks referencing removed file
        let post2_id = &files_before["blog/post2.md"];
        assert!(!chunks_after.iter().any(|(_, fid)| fid == post2_id));
    }

    #[tokio::test]
    async fn incremental_frontmatter_only_change() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (_, chunks_before) = read_index_state(tmp.path());
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Change only frontmatter (add a tag), keep same body
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\n  - new-tag\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (_, chunks_after) = read_index_state(tmp.path());
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // Chunks preserved — body didn't change, no re-embedding
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn force_full_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            false,
            false,
            true,
            false, // skip_gitignore
            false, // verbose
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let old_file_ids: HashSet<String> = files_before.values().cloned().collect();
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Force rebuild — should generate all new IDs
        let output = run(tmp.path(), None, None, None, true, true, false).await;
        assert!(!output.has_failed_step());

        let (files_after, chunks_after) = read_index_state(tmp.path());
        let new_file_ids: HashSet<String> = files_after.values().cloned().collect();
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // All IDs should be different (new UUIDs)
        assert!(
            old_file_ids.is_disjoint(&new_file_ids),
            "force rebuild should generate new file_ids"
        );
        assert!(
            old_chunk_ids.is_disjoint(&new_chunk_ids),
            "force rebuild should generate new chunk_ids"
        );
    }

    #[test]
    fn normalize_revision_clears_empty_and_none() {
        assert_eq!(normalize_revision(""), None);
        assert_eq!(normalize_revision("None"), None);
        assert_eq!(normalize_revision("none"), None);
        assert_eq!(normalize_revision("NONE"), None);
        assert_eq!(normalize_revision("abc123"), Some("abc123".to_string()));
        assert_eq!(
            normalize_revision("abc123def"),
            Some("abc123def".to_string())
        );
    }
}
