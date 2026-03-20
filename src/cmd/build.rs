use crate::discover::field_type::FieldType;
use crate::discover::scan::{ScannedFile, ScannedFiles};
use crate::index::backend::Backend;
use crate::index::chunk::{extract_plain_text, Chunks};
use crate::index::embed::{Embedder, ModelConfig};
use crate::index::storage::{content_hash, BuildMetadata, ChunkRow, FileIndexEntry, FileRow};
use crate::outcome::commands::BuildOutcome;
use crate::outcome::{
    ClassifyOutcome, EmbedFilesOutcome, LoadModelOutcome, Outcome, ReadConfigOutcome, ScanOutcome,
    ValidateOutcome, WriteIndexOutcome,
};
use crate::output::BuildFileDetail;
use crate::schema::config::{BuildConfig, MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use crate::step::{ErrorKind, Step, StepError, StepOutcome};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

// ============================================================================
// Classification types + logic (moved from pipeline/classify.rs)
// ============================================================================

/// A file that needs chunking and embedding.
struct FileToEmbed<'a> {
    /// Unique file identifier (preserved for edited files, new UUID for new files).
    file_id: String,
    /// Reference to the scanned file data.
    scanned: &'a ScannedFile,
}

/// Data produced by classification, carried forward to embed and write_index steps.
struct ClassifyData<'a> {
    /// Whether this is a full rebuild.
    full_rebuild: bool,
    /// Files that need chunking + embedding (new or edited).
    needs_embedding: Vec<FileToEmbed<'a>>,
    /// Maps filename → file_id for ALL current files (new, edited, unchanged).
    file_id_map: HashMap<String, String>,
    /// Chunks retained from unchanged files.
    retained_chunks: Vec<ChunkRow>,
    /// Number of files removed since previous build.
    removed_count: usize,
    /// Number of chunks dropped from removed files.
    chunks_removed: usize,
    /// Per-file chunk counts for removed files (for verbose output).
    removed_details: Vec<BuildFileDetail>,
}

struct FileClassification<'a> {
    needs_embedding: Vec<FileToEmbed<'a>>,
    file_id_map: HashMap<String, String>,
    unchanged_file_ids: HashSet<String>,
    removed_count: usize,
    removed_file_ids: HashSet<String>,
    removed_filenames: Vec<String>,
}

fn classify_files<'a>(
    scanned: &'a ScannedFiles,
    existing_index: &[FileIndexEntry],
) -> FileClassification<'a> {
    let existing: HashMap<&str, (&str, &str)> = existing_index
        .iter()
        .map(|e| {
            (
                e.filename.as_str(),
                (e.file_id.as_str(), e.content_hash.as_str()),
            )
        })
        .collect();

    let mut needs_embedding = Vec::new();
    let mut file_id_map = HashMap::new();
    let mut unchanged_file_ids = HashSet::new();
    let mut seen_filenames = HashSet::new();

    for file in &scanned.files {
        let filename = file.path.display().to_string();
        let hash = content_hash(&file.content);

        if let Some(&(old_id, old_hash)) = existing.get(filename.as_str()) {
            seen_filenames.insert(filename.clone());
            if hash == old_hash {
                file_id_map.insert(filename, old_id.to_string());
                unchanged_file_ids.insert(old_id.to_string());
            } else {
                let file_id = old_id.to_string();
                file_id_map.insert(filename, file_id.clone());
                needs_embedding.push(FileToEmbed {
                    file_id,
                    scanned: file,
                });
            }
        } else {
            let file_id = uuid::Uuid::new_v4().to_string();
            file_id_map.insert(filename, file_id.clone());
            needs_embedding.push(FileToEmbed {
                file_id,
                scanned: file,
            });
        }
    }

    let mut removed_file_ids = HashSet::new();
    let mut removed_filenames = Vec::new();
    for entry in existing_index {
        if !seen_filenames.contains(entry.filename.as_str()) {
            removed_file_ids.insert(entry.file_id.clone());
            removed_filenames.push(entry.filename.clone());
        }
    }
    let removed_count = removed_filenames.len();

    FileClassification {
        needs_embedding,
        file_id_map,
        unchanged_file_ids,
        removed_count,
        removed_file_ids,
        removed_filenames,
    }
}

// ============================================================================
// Embed logic (moved from pipeline/embed.rs)
// ============================================================================

/// Data produced by the embed files step.
struct EmbedFilesData {
    /// Chunk rows for newly embedded files.
    chunk_rows: Vec<ChunkRow>,
    /// Per-file chunk counts (for verbose output).
    details: Vec<BuildFileDetail>,
}

/// Chunk, extract plain text, embed, and produce chunk rows for a single file.
async fn embed_file(
    file_id: &str,
    file: &ScannedFile,
    max_chunk_size: usize,
    embedder: &Embedder,
) -> Vec<ChunkRow> {
    let chunks = Chunks::new(&file.content, max_chunk_size);
    let plain_texts: Vec<String> = chunks
        .iter()
        .map(|c| extract_plain_text(&c.plain_text))
        .collect();
    let text_refs: Vec<&str> = plain_texts.iter().map(|s| s.as_str()).collect();
    let embeddings = if text_refs.is_empty() {
        vec![]
    } else {
        embedder.embed_batch(&text_refs).await
    };

    chunks
        .iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| ChunkRow {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            chunk_index: chunk.chunk_index as i32,
            start_line: chunk.start_line as i32,
            end_line: chunk.end_line as i32,
            embedding,
        })
        .collect()
}

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

    // 1. Read config — calls MdvsToml::read() + validate() directly
    let config_start = Instant::now();
    let config_path_buf = path.join("mdvs.toml");
    let config = match MdvsToml::read(&config_path_buf) {
        Ok(cfg) => match cfg.validate() {
            Ok(()) => {
                substeps.push(Step::leaf(
                    Outcome::ReadConfig(ReadConfigOutcome {
                        config_path: config_path_buf.display().to_string(),
                    }),
                    config_start.elapsed().as_millis() as u64,
                ));
                Some(cfg)
            }
            Err(e) => {
                substeps.push(Step::failed(
                    ErrorKind::User,
                    format!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'"),
                    config_start.elapsed().as_millis() as u64,
                ));
                None
            }
        },
        Err(e) => {
            substeps.push(Step::failed(
                ErrorKind::User,
                e.to_string(),
                config_start.elapsed().as_millis() as u64,
            ));
            None
        }
    };
    let config = match config {
        Some(c) => c,
        None => return fail_from_last(&mut substeps, start),
    };

    // 2. Auto-update (conditional)
    let should_update = !no_update && config.build.as_ref().is_some_and(|b| b.auto_update);
    if should_update {
        let update_step = crate::cmd::update::run(path, &[], false, false, false).await;
        if crate::step::has_failed(&update_step) {
            substeps.push(update_step);
            return fail_msg(&mut substeps, start, ErrorKind::User, "auto-update failed");
        }
        substeps.push(update_step);
    }

    // Re-read config if auto-update ran
    let mut config = if should_update {
        let re_read_start = Instant::now();
        let re_read_path = path.join("mdvs.toml");
        match MdvsToml::read(&re_read_path) {
            Ok(cfg) => match cfg.validate() {
                Ok(()) => {
                    substeps.push(Step::leaf(
                        Outcome::ReadConfig(ReadConfigOutcome {
                            config_path: re_read_path.display().to_string(),
                        }),
                        re_read_start.elapsed().as_millis() as u64,
                    ));
                    cfg
                }
                Err(e) => {
                    substeps.push(Step::failed(
                        ErrorKind::User,
                        format!(
                            "mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'"
                        ),
                        re_read_start.elapsed().as_millis() as u64,
                    ));
                    return fail_from_last(&mut substeps, start);
                }
            },
            Err(e) => {
                substeps.push(Step::failed(
                    ErrorKind::User,
                    e.to_string(),
                    re_read_start.elapsed().as_millis() as u64,
                ));
                return fail_from_last(&mut substeps, start);
            }
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
        return fail_from_last(&mut substeps, start);
    }

    let scan_start = Instant::now();
    let scanned = match ScannedFiles::scan(path, &config.scan) {
        Ok(s) => {
            substeps.push(Step::leaf(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: config.scan.glob.clone(),
                }),
                scan_start.elapsed().as_millis() as u64,
            ));
            s
        }
        Err(e) => {
            substeps.push(Step::failed(
                ErrorKind::Application,
                e.to_string(),
                scan_start.elapsed().as_millis() as u64,
            ));
            return fail_from_last(&mut substeps, start);
        }
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
        return fail_from_last(&mut substeps, start);
    }

    let validate_start = Instant::now();
    let check_result = match crate::cmd::check::validate(&scanned, &config, false) {
        Ok(r) => r,
        Err(e) => {
            substeps.push(Step::failed(
                ErrorKind::Application,
                e.to_string(),
                validate_start.elapsed().as_millis() as u64,
            ));
            return fail_from_last(&mut substeps, start);
        }
    };
    substeps.push(Step::leaf(
        Outcome::Validate(ValidateOutcome {
            files_checked: check_result.files_checked,
            violations: check_result.field_violations.clone(),
            new_fields: check_result.new_fields.clone(),
        }),
        validate_start.elapsed().as_millis() as u64,
    ));
    let violations = check_result.field_violations;
    let new_fields = check_result.new_fields;

    // Abort on violations
    if !violations.is_empty() {
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
            return fail_from_last(&mut substeps, start);
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
            return fail_from_last(&mut substeps, start);
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
            return fail_from_last(&mut substeps, start);
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
        return fail_from_last(&mut substeps, start);
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
                return fail_from_last(&mut substeps, start);
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
                return fail_from_last(&mut substeps, start);
            }
        }
    };

    let classify_start = Instant::now();
    let classify_data = if full_rebuild {
        let mut file_id_map = HashMap::new();
        let needs_embedding: Vec<FileToEmbed<'_>> = scanned
            .files
            .iter()
            .map(|f| {
                let file_id = uuid::Uuid::new_v4().to_string();
                let filename = f.path.display().to_string();
                file_id_map.insert(filename, file_id.clone());
                FileToEmbed {
                    file_id,
                    scanned: f,
                }
            })
            .collect();
        let count = needs_embedding.len();
        substeps.push(Step::leaf(
            Outcome::Classify(ClassifyOutcome {
                full_rebuild: true,
                needs_embedding: count,
                unchanged: 0,
                removed: 0,
            }),
            classify_start.elapsed().as_millis() as u64,
        ));
        ClassifyData {
            full_rebuild: true,
            needs_embedding,
            file_id_map,
            retained_chunks: vec![],
            removed_count: 0,
            chunks_removed: 0,
            removed_details: vec![],
        }
    } else {
        let classification = classify_files(&scanned, &existing_index);

        let mut removed_chunk_counts: HashMap<&str, usize> = HashMap::new();
        for c in &existing_chunks {
            if classification.removed_file_ids.contains(&c.file_id) {
                *removed_chunk_counts.entry(c.file_id.as_str()).or_default() += 1;
            }
        }
        let chunks_removed: usize = removed_chunk_counts.values().sum();

        let filename_to_id: HashMap<&str, &str> = existing_index
            .iter()
            .map(|e| (e.filename.as_str(), e.file_id.as_str()))
            .collect();
        let mut removed_details = Vec::new();
        for filename in &classification.removed_filenames {
            let file_id = filename_to_id.get(filename.as_str()).copied().unwrap_or("");
            let chunk_count = removed_chunk_counts.get(file_id).copied().unwrap_or(0);
            removed_details.push(BuildFileDetail {
                filename: filename.clone(),
                chunks: chunk_count,
            });
        }

        let retained_chunks: Vec<ChunkRow> = existing_chunks
            .into_iter()
            .filter(|c| classification.unchanged_file_ids.contains(&c.file_id))
            .collect();

        let needs_count = classification.needs_embedding.len();
        let unchanged_count = classification.unchanged_file_ids.len();
        let removed_count = classification.removed_count;

        substeps.push(Step::leaf(
            Outcome::Classify(ClassifyOutcome {
                full_rebuild: false,
                needs_embedding: needs_count,
                unchanged: unchanged_count,
                removed: removed_count,
            }),
            classify_start.elapsed().as_millis() as u64,
        ));
        ClassifyData {
            full_rebuild: false,
            needs_embedding: classification.needs_embedding,
            file_id_map: classification.file_id_map,
            retained_chunks,
            removed_count,
            chunks_removed,
            removed_details,
        }
    };

    let needs_embedding = !classify_data.needs_embedding.is_empty();

    // 6. Load model — calls ModelConfig::try_from() + Embedder::load() directly
    let embedder = if needs_embedding {
        let model_start = Instant::now();
        match ModelConfig::try_from(embedding) {
            Ok(mc) => match Embedder::load(&mc) {
                Ok(emb) => {
                    substeps.push(Step::leaf(
                        Outcome::LoadModel(LoadModelOutcome {
                            model_name: embedding.name.clone(),
                            dimension: emb.dimension(),
                        }),
                        model_start.elapsed().as_millis() as u64,
                    ));
                    Some(emb)
                }
                Err(e) => {
                    substeps.push(Step::failed(
                        ErrorKind::Application,
                        e.to_string(),
                        model_start.elapsed().as_millis() as u64,
                    ));
                    None
                }
            },
            Err(e) => {
                substeps.push(Step::failed(
                    ErrorKind::Application,
                    e.to_string(),
                    model_start.elapsed().as_millis() as u64,
                ));
                None
            }
        }
    } else {
        substeps.push(Step::skipped());
        None
    };

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
        return fail_msg(
            &mut substeps,
            start,
            ErrorKind::Application,
            "model loading failed",
        );
    }

    // 7. Embed files
    let max_chunk_size = chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    if let Some(msg) = dim_error {
        substeps.push(Step::failed(ErrorKind::User, msg, 0));
        return fail_from_last(&mut substeps, start);
    }

    let embed_data = if needs_embedding {
        let embed_start = Instant::now();
        let emb = embedder.as_ref().unwrap();
        let mut embed_chunk_rows = Vec::new();
        let mut details = Vec::new();
        for fte in &classify_data.needs_embedding {
            let crs = embed_file(&fte.file_id, fte.scanned, max_chunk_size, emb).await;
            details.push(BuildFileDetail {
                filename: fte.scanned.path.display().to_string(),
                chunks: crs.len(),
            });
            embed_chunk_rows.extend(crs);
        }
        substeps.push(Step::leaf(
            Outcome::EmbedFiles(EmbedFilesOutcome {
                files_embedded: classify_data.needs_embedding.len(),
                chunks_produced: embed_chunk_rows.len(),
            }),
            embed_start.elapsed().as_millis() as u64,
        ));
        Some(EmbedFilesData {
            chunk_rows: embed_chunk_rows,
            details,
        })
    } else {
        substeps.push(Step::skipped());
        None
    };

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

    let write_start = Instant::now();
    match backend.write_index(&schema_fields, &file_rows, &chunk_rows, build_meta) {
        Ok(()) => {
            substeps.push(Step::leaf(
                Outcome::WriteIndex(WriteIndexOutcome {
                    files_written: file_rows.len(),
                    chunks_written: chunk_rows.len(),
                }),
                write_start.elapsed().as_millis() as u64,
            ));
        }
        Err(e) => {
            substeps.push(Step::failed(
                ErrorKind::Application,
                e.to_string(),
                write_start.elapsed().as_millis() as u64,
            ));
            return fail_from_last(&mut substeps, start);
        }
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

/// Extract error from last failed substep and return a failed command Step.
fn fail_from_last(substeps: &mut Vec<Step<Outcome>>, start: Instant) -> Step<Outcome> {
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

/// Return a failed command Step with a specific message.
fn fail_msg(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    kind: ErrorKind,
    msg: &str,
) -> Step<Outcome> {
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
        assert!(crate::step::has_failed(&output));
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
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            !crate::step::has_failed(&output),
            "first build failed: {:#?}",
            output
        );

        // Run build again (tests standalone rebuild)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            !crate::step::has_failed(&output),
            "build failed: {:#?}",
            output
        );
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(crate::step::has_failed(&output));
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
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
            !crate::step::has_failed(&output),
            "expected success with --force, got failed step"
        );
        assert!(!crate::step::has_failed(&output));
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
        assert!(!crate::step::has_failed(&output));

        // Verify no model/chunking sections (auto-flag sections are present from init)
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());

        // Build should fill defaults and succeed
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(
            !crate::step::has_failed(&output),
            "build failed: {:#?}",
            output
        );

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
        assert!(!crate::step::has_failed(&output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(crate::step::has_failed(&output));
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
        assert!(!crate::step::has_failed(&output));

        let output = run(tmp.path(), None, None, Some(512), false, true, false).await;
        assert!(crate::step::has_failed(&output));
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
        assert!(!crate::step::has_failed(&output));

        // Change chunk size with --force (same model so no dimension mismatch)
        let output = run(tmp.path(), None, None, Some(512), true, true, false).await;
        assert!(
            !crate::step::has_failed(&output),
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
        assert!(!crate::step::has_failed(&output));

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("config changed since last build"));
        assert!(err.message.contains("chunk_size"));

        // Build with --force should succeed
        let output = run(tmp.path(), None, None, None, true, true, false).await;
        assert!(
            !crate::step::has_failed(&output),
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
            !crate::step::has_failed(&output),
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
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path());

        // Build again with no changes
        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path());
        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: Third\ntags:\n  - new\ndraft: false\n---\n# Third\nNew post content.",
        )
        .unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

        let (files_before, _) = read_index_state(tmp.path());
        assert_eq!(files_before.len(), 2);

        // Remove post2
        fs::remove_file(tmp.path().join("blog/post2.md")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

        let (_, chunks_before) = read_index_state(tmp.path());
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Change only frontmatter (add a tag), keep same body
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\n  - new-tag\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));

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
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let old_file_ids: HashSet<String> = files_before.values().cloned().collect();
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Force rebuild — should generate all new IDs
        let output = run(tmp.path(), None, None, None, true, true, false).await;
        assert!(!crate::step::has_failed(&output));

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

    // ========================================================================
    // classify_files unit tests (moved from pipeline/classify.rs)
    // ========================================================================

    use crate::discover::scan::ScannedFile;

    fn make_scanned_files(files: Vec<(&str, &str)>) -> ScannedFiles {
        ScannedFiles {
            files: files
                .into_iter()
                .map(|(path, body)| ScannedFile {
                    path: std::path::PathBuf::from(path),
                    data: None,
                    content: body.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn classify_all_new() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing: Vec<FileIndexEntry> = vec![];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 2);
        assert_eq!(c.unchanged_file_ids.len(), 0);
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map.len(), 2);
    }

    #[test]
    fn classify_all_unchanged() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("hello"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("world"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 0);
        assert_eq!(c.unchanged_file_ids.len(), 2);
        assert!(c.unchanged_file_ids.contains("f1"));
        assert!(c.unchanged_file_ids.contains("f2"));
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map["a.md"], "f1");
        assert_eq!(c.file_id_map["b.md"], "f2");
    }

    #[test]
    fn classify_mixed() {
        let scanned = make_scanned_files(vec![
            ("a.md", "same content"),
            ("b.md", "new body"),
            ("c.md", "brand new"),
        ]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("same content"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("old body"),
            },
            FileIndexEntry {
                file_id: "f3".into(),
                filename: "d.md".into(),
                content_hash: content_hash("deleted"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        assert!(c.unchanged_file_ids.contains("f1"));
        assert_eq!(c.file_id_map["a.md"], "f1");

        assert_eq!(c.needs_embedding.len(), 2);
        let edited = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("b.md"))
            .unwrap();
        assert_eq!(edited.file_id, "f2");

        let new = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("c.md"))
            .unwrap();
        assert_ne!(new.file_id, "f1");
        assert_ne!(new.file_id, "f2");
        assert_ne!(new.file_id, "f3");

        assert_eq!(c.removed_count, 1);
        assert!(!c.file_id_map.contains_key("d.md"));
    }
}
