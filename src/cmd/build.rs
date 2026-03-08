use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{content_hash, BuildMetadata, FileRow};
use crate::output::{format_file_count, format_json_compact, CommandOutput, NewField};
use crate::pipeline::classify::{run_classify, ClassifyOutput};
use crate::pipeline::embed::{run_embed_files, EmbedFilesOutput};
use crate::pipeline::load_model::{run_load_model, LoadModelOutput};
use crate::pipeline::read_config::{run_read_config, ReadConfigOutput};
use crate::pipeline::scan::{run_scan, ScanOutput};
use crate::pipeline::validate::{run_validate, ValidateOutput};
use crate::pipeline::write_index::{run_write_index, BuildFileDetail, WriteIndexOutput};
use crate::pipeline::{ErrorKind, ProcessingStepError, ProcessingStepResult};
use crate::schema::config::{MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::path::Path;
use tracing::instrument;

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

// ============================================================================
// BuildResult
// ============================================================================

/// Result of the `build` command: embedding and index statistics.
#[derive(Debug, Serialize)]
pub struct BuildResult {
    /// Whether this was a full rebuild (vs incremental).
    pub full_rebuild: bool,
    /// Total number of files in the final index.
    pub files_total: usize,
    /// Number of files that were chunked and embedded this run.
    pub files_embedded: usize,
    /// Number of files reused from the previous index (content unchanged).
    pub files_unchanged: usize,
    /// Number of files removed since the last build.
    pub files_removed: usize,
    /// Total number of chunks in the final index.
    pub chunks_total: usize,
    /// Number of chunks produced by newly embedded files.
    pub chunks_embedded: usize,
    /// Number of chunks retained from unchanged files.
    pub chunks_unchanged: usize,
    /// Number of chunks dropped from removed files.
    pub chunks_removed: usize,
    /// Fields found in frontmatter but not yet in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
    /// Per-file chunk counts for embedded files (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_files: Option<Vec<BuildFileDetail>>,
    /// Per-file chunk counts for removed files (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_files: Option<Vec<BuildFileDetail>>,
}

fn format_chunk_count(n: usize) -> String {
    if n == 1 {
        "1 chunk".to_string()
    } else {
        format!("{n} chunks")
    }
}

impl CommandOutput for BuildResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // New fields (shown before stats)
        for nf in &self.new_fields {
            out.push_str(&format!(
                "  new field: {} ({})\n",
                nf.name,
                format_file_count(nf.files_found)
            ));
        }
        if !self.new_fields.is_empty() {
            out.push_str("Run 'mdvs update' to incorporate new fields.\n\n");
        }

        // One-liner
        let rebuild_suffix = if self.full_rebuild {
            " (full rebuild)"
        } else {
            ""
        };
        out.push_str(&format!(
            "Built index — {}, {}{rebuild_suffix}\n",
            format_file_count(self.files_total),
            format_chunk_count(self.chunks_total)
        ));

        // Stats table
        out.push('\n');
        if verbose {
            // Verbose: record tables for embedded/removed, compact for unchanged
            if self.files_embedded > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "embedded".to_string(),
                    format_file_count(self.files_embedded),
                    format_chunk_count(self.chunks_embedded),
                ]);
                let detail = match &self.embedded_files {
                    Some(files) => {
                        let lines: Vec<String> = files
                            .iter()
                            .map(|f| {
                                format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks))
                            })
                            .collect();
                        lines.join("\n")
                    }
                    None => String::new(),
                };
                builder.push_record([detail, String::new(), String::new()]);
                let mut table = builder.build();
                style_record(&mut table, 3);
                out.push_str(&format!("{table}\n"));
            }
            if self.files_unchanged > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "unchanged".to_string(),
                    format_file_count(self.files_unchanged),
                    format_chunk_count(self.chunks_unchanged),
                ]);
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
            if self.files_removed > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "removed".to_string(),
                    format_file_count(self.files_removed),
                    format_chunk_count(self.chunks_removed),
                ]);
                let detail = match &self.removed_files {
                    Some(files) => {
                        let lines: Vec<String> = files
                            .iter()
                            .map(|f| {
                                format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks))
                            })
                            .collect();
                        lines.join("\n")
                    }
                    None => String::new(),
                };
                builder.push_record([detail, String::new(), String::new()]);
                let mut table = builder.build();
                style_record(&mut table, 3);
                out.push_str(&format!("{table}\n"));
            }
        } else {
            // Compact: single table with all non-zero categories
            let mut builder = Builder::default();
            if self.files_embedded > 0 {
                builder.push_record([
                    "embedded".to_string(),
                    format_file_count(self.files_embedded),
                    format_chunk_count(self.chunks_embedded),
                ]);
            }
            if self.files_unchanged > 0 {
                builder.push_record([
                    "unchanged".to_string(),
                    format_file_count(self.files_unchanged),
                    format_chunk_count(self.chunks_unchanged),
                ]);
            }
            if self.files_removed > 0 {
                builder.push_record([
                    "removed".to_string(),
                    format_file_count(self.files_removed),
                    format_chunk_count(self.chunks_removed),
                ]);
            }
            let mut table = builder.build();
            style_compact(&mut table);
            out.push_str(&format!("{table}\n"));
        }

        out
    }
}

// ============================================================================
// BuildCommandOutput (pipeline)
// ============================================================================

/// Step records for each phase of the build pipeline.
#[derive(Debug, Serialize)]
pub struct BuildProcessOutput {
    /// Read and parse `mdvs.toml`.
    pub read_config: ProcessingStepResult<ReadConfigOutput>,
    /// Scan the project directory for markdown files.
    pub scan: ProcessingStepResult<ScanOutput>,
    /// Validate frontmatter against the schema.
    pub validate: ProcessingStepResult<ValidateOutput>,
    /// Classify files as new/edited/unchanged/removed.
    pub classify: ProcessingStepResult<ClassifyOutput>,
    /// Load the embedding model.
    pub load_model: ProcessingStepResult<LoadModelOutput>,
    /// Embed files that need embedding.
    pub embed_files: ProcessingStepResult<EmbedFilesOutput>,
    /// Write the index to disk.
    pub write_index: ProcessingStepResult<WriteIndexOutput>,
}

/// Complete output of the `build` command.
#[derive(Debug, Serialize)]
pub struct BuildCommandOutput {
    /// Step-by-step process records.
    pub process: BuildProcessOutput,
    /// Build statistics (present when build completes successfully).
    pub result: Option<BuildResult>,
}

impl BuildCommandOutput {
    /// Returns `true` if any step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.read_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.scan, ProcessingStepResult::Failed(_))
            || matches!(self.process.validate, ProcessingStepResult::Failed(_))
            || matches!(self.process.classify, ProcessingStepResult::Failed(_))
            || matches!(self.process.load_model, ProcessingStepResult::Failed(_))
            || matches!(self.process.embed_files, ProcessingStepResult::Failed(_))
            || matches!(self.process.write_index, ProcessingStepResult::Failed(_))
    }

    /// Returns `true` if validation found violations (build aborted).
    pub fn has_violations(&self) -> bool {
        match &self.process.validate {
            ProcessingStepResult::Completed(step) => step.output.violation_count > 0,
            _ => false,
        }
    }
}

impl CommandOutput for BuildCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        if self.has_violations() {
            // Validation found violations — show step lines + violation message
            let violation_msg = match &self.process.validate {
                ProcessingStepResult::Completed(step) => {
                    format!(
                        "Build aborted — {} violation(s) found. Run `mdvs check` for details.\n",
                        step.output.violation_count
                    )
                }
                _ => "Build aborted — validation failed.\n".to_string(),
            };
            if verbose {
                let mut out = String::new();
                out.push_str(&format!("{}\n", self.process.read_config.format_line()));
                out.push_str(&format!("{}\n", self.process.scan.format_line()));
                out.push_str(&format!("{}\n", self.process.validate.format_line()));
                out.push('\n');
                out.push_str(&violation_msg);
                out
            } else {
                violation_msg
            }
        } else if let Some(result) = &self.result {
            if verbose {
                let mut out = String::new();
                out.push_str(&format!("{}\n", self.process.read_config.format_line()));
                out.push_str(&format!("{}\n", self.process.scan.format_line()));
                out.push_str(&format!("{}\n", self.process.validate.format_line()));
                out.push_str(&format!("{}\n", self.process.classify.format_line()));
                out.push_str(&format!("{}\n", self.process.load_model.format_line()));
                out.push_str(&format!("{}\n", self.process.embed_files.format_line()));
                out.push_str(&format!("{}\n", self.process.write_index.format_line()));
                out.push('\n');
                out.push_str(&result.format_text(verbose));
                out
            } else {
                result.format_text(verbose)
            }
        } else {
            // Pipeline didn't complete — show steps up to the failure
            let mut out = String::new();
            out.push_str(&format!("{}\n", self.process.read_config.format_line()));
            if !matches!(self.process.scan, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.scan.format_line()));
            }
            if !matches!(self.process.validate, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.validate.format_line()));
            }
            if !matches!(self.process.classify, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.classify.format_line()));
            }
            if !matches!(self.process.load_model, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.load_model.format_line()));
            }
            if !matches!(self.process.embed_files, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.embed_files.format_line()));
            }
            if !matches!(self.process.write_index, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.write_index.format_line()));
            }
            out
        }
    }
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
    verbose: bool,
) -> BuildCommandOutput {
    // 1. read_config
    let (read_config_step, config) = run_read_config(path);
    let mut config = match config {
        Some(c) => c,
        None => {
            return BuildCommandOutput {
                process: BuildProcessOutput {
                    read_config: read_config_step,
                    scan: ProcessingStepResult::Skipped,
                    validate: ProcessingStepResult::Skipped,
                    classify: ProcessingStepResult::Skipped,
                    load_model: ProcessingStepResult::Skipped,
                    embed_files: ProcessingStepResult::Skipped,
                    write_index: ProcessingStepResult::Skipped,
                },
                result: None,
            };
        }
    };

    // Config mutation (inline, not a step)
    // Fill missing build sections, apply --set-* flags
    let mutation_error = mutate_config(
        &mut config,
        path,
        set_model,
        set_revision,
        set_chunk_size,
        force,
    );

    // 2. scan
    let (scan_step, scanned) = if let Some(msg) = mutation_error {
        (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: msg,
            }),
            None,
        )
    } else {
        run_scan(path, &config.scan)
    };

    let scanned = match scanned {
        Some(s) => s,
        None => {
            return BuildCommandOutput {
                process: BuildProcessOutput {
                    read_config: read_config_step,
                    scan: scan_step,
                    validate: ProcessingStepResult::Skipped,
                    classify: ProcessingStepResult::Skipped,
                    load_model: ProcessingStepResult::Skipped,
                    embed_files: ProcessingStepResult::Skipped,
                    write_index: ProcessingStepResult::Skipped,
                },
                result: None,
            };
        }
    };

    // 3. validate (no-files check lands here as pre-check)
    let (validate_step, validation_data) = if scanned.files.is_empty() {
        (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: format!("no markdown files found in '{}'", path.display()),
            }),
            None,
        )
    } else {
        run_validate(&scanned, &config, false)
    };

    let validation_data = match validation_data {
        Some(d) => d,
        None => {
            return BuildCommandOutput {
                process: BuildProcessOutput {
                    read_config: read_config_step,
                    scan: scan_step,
                    validate: validate_step,
                    classify: ProcessingStepResult::Skipped,
                    load_model: ProcessingStepResult::Skipped,
                    embed_files: ProcessingStepResult::Skipped,
                    write_index: ProcessingStepResult::Skipped,
                },
                result: None,
            };
        }
    };

    let (violations, new_fields) = validation_data;
    let has_violations = !violations.is_empty();

    // If violations found, abort — remaining steps skipped
    if has_violations {
        return BuildCommandOutput {
            process: BuildProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                validate: validate_step,
                classify: ProcessingStepResult::Skipped,
                load_model: ProcessingStepResult::Skipped,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // Convert schema fields
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
            return BuildCommandOutput {
                process: BuildProcessOutput {
                    read_config: read_config_step,
                    scan: scan_step,
                    validate: validate_step,
                    classify: ProcessingStepResult::Failed(ProcessingStepError {
                        kind: ErrorKind::Application,
                        message: msg,
                    }),
                    load_model: ProcessingStepResult::Skipped,
                    embed_files: ProcessingStepResult::Skipped,
                    write_index: ProcessingStepResult::Skipped,
                },
                result: None,
            };
        }
    };

    let embedding = config.embedding_model.as_ref().unwrap();
    let chunking = config.chunking.as_ref().unwrap();
    let backend = Backend::parquet(path, config.internal_prefix());

    // Config change detection (pre-check for classify step)
    let config_change_error = detect_config_changes(&backend, embedding, chunking, &config, force);

    // 4. classify
    let full_rebuild = force || !backend.exists();

    let (classify_step, classify_data) = if let Some(msg) = config_change_error {
        (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: msg,
            }),
            None,
        )
    } else {
        // Read existing index for classification
        let existing_index = if full_rebuild {
            vec![]
        } else {
            match backend.read_file_index() {
                Ok(idx) => idx,
                Err(e) => {
                    return BuildCommandOutput {
                        process: BuildProcessOutput {
                            read_config: read_config_step,
                            scan: scan_step,
                            validate: validate_step,
                            classify: ProcessingStepResult::Failed(ProcessingStepError {
                                kind: ErrorKind::Application,
                                message: e.to_string(),
                            }),
                            load_model: ProcessingStepResult::Skipped,
                            embed_files: ProcessingStepResult::Skipped,
                            write_index: ProcessingStepResult::Skipped,
                        },
                        result: None,
                    };
                }
            }
        };
        let existing_chunks = if full_rebuild {
            vec![]
        } else {
            match backend.read_chunk_rows() {
                Ok(crs) => crs,
                Err(e) => {
                    return BuildCommandOutput {
                        process: BuildProcessOutput {
                            read_config: read_config_step,
                            scan: scan_step,
                            validate: validate_step,
                            classify: ProcessingStepResult::Failed(ProcessingStepError {
                                kind: ErrorKind::Application,
                                message: e.to_string(),
                            }),
                            load_model: ProcessingStepResult::Skipped,
                            embed_files: ProcessingStepResult::Skipped,
                            write_index: ProcessingStepResult::Skipped,
                        },
                        result: None,
                    };
                }
            }
        };
        run_classify(&scanned, &existing_index, existing_chunks, full_rebuild)
    };

    let classify_data = match classify_data {
        Some(d) => d,
        None => {
            return BuildCommandOutput {
                process: BuildProcessOutput {
                    read_config: read_config_step,
                    scan: scan_step,
                    validate: validate_step,
                    classify: classify_step,
                    load_model: ProcessingStepResult::Skipped,
                    embed_files: ProcessingStepResult::Skipped,
                    write_index: ProcessingStepResult::Skipped,
                },
                result: None,
            };
        }
    };

    let needs_embedding = !classify_data.needs_embedding.is_empty();

    // 5. load_model (skip if nothing to embed)
    let (load_model_step, embedder) = if needs_embedding {
        run_load_model(embedding)
    } else {
        (ProcessingStepResult::Skipped, None)
    };

    // Dimension check (pre-check for embed_files)
    let dim_error = match &embedder {
        Some(emb) => match backend.embedding_dimension() {
            Ok(Some(existing_dim)) => {
                let model_dim = emb.dimension() as i32;
                if existing_dim != model_dim {
                    Some(format!(
                        "dimension mismatch: model produces {model_dim}-dim embeddings but existing index has {existing_dim}-dim"
                    ))
                } else {
                    None
                }
            }
            Ok(None) => None,
            Err(e) => Some(e.to_string()),
        },
        None if needs_embedding => {
            // load_model failed — embed_files will be skipped via embedder check below
            None
        }
        None => None,
    };

    // If load_model failed, skip embed_files and write_index
    if needs_embedding && embedder.is_none() {
        return BuildCommandOutput {
            process: BuildProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // 6. embed_files
    let max_chunk_size = chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    let (embed_files_step, embed_data) = if let Some(msg) = dim_error {
        (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: msg,
            }),
            None,
        )
    } else if needs_embedding {
        let emb = embedder.as_ref().unwrap();
        run_embed_files(&classify_data.needs_embedding, emb, max_chunk_size).await
    } else {
        (ProcessingStepResult::Skipped, None)
    };

    // If embed_files failed, skip write_index
    if needs_embedding
        && embed_data.is_none()
        && !matches!(embed_files_step, ProcessingStepResult::Skipped)
    {
        return BuildCommandOutput {
            process: BuildProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: embed_files_step,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // 7. write_index — assemble file_rows + chunk_rows from classify_data + embed_data
    // File rows: always built fresh from scanned files with file_ids from classify
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

    // Chunk rows: retained chunks + newly embedded chunks
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
        internal_prefix: config.internal_prefix().to_string(),
    };

    let write_index_step = run_write_index(
        &backend,
        &schema_fields,
        &file_rows,
        &chunk_rows,
        build_meta,
    );

    if matches!(write_index_step, ProcessingStepResult::Failed(_)) {
        return BuildCommandOutput {
            process: BuildProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: embed_files_step,
                write_index: write_index_step,
            },
            result: None,
        };
    }

    // Assemble BuildResult
    let chunks_embedded: usize = embedded_details.iter().map(|d| d.chunks).sum();
    let chunks_total = chunk_rows.len();
    let chunks_unchanged = chunks_total - chunks_embedded;

    let result = BuildResult {
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
        embedded_files: if verbose {
            Some(embedded_details)
        } else {
            None
        },
        removed_files: if verbose && !classify_data.removed_details.is_empty() {
            Some(classify_data.removed_details)
        } else {
            None
        },
    };

    BuildCommandOutput {
        process: BuildProcessOutput {
            read_config: read_config_step,
            scan: scan_step,
            validate: validate_step,
            classify: classify_step,
            load_model: load_model_step,
            embed_files: embed_files_step,
            write_index: write_index_step,
        },
        result: Some(result),
    }
}

// ============================================================================
// Helpers
// ============================================================================

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
                revision: set_revision.map(|s| s.to_string()),
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
                em.revision = Some(r.to_string());
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
            ch.max_chunk_size = set_chunk_size.unwrap();
            config_changed = true;
        }
        Some(_) => {}
    }

    if config.search.is_none() {
        config.search = Some(SearchConfig { default_limit: 10 });
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
    config: &MdvsToml,
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
    if meta.internal_prefix != config.internal_prefix() {
        mismatches.push(format!(
            "internal_prefix: '{}' -> '{}'",
            meta.internal_prefix,
            config.internal_prefix(),
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
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(output.has_failed_step());
        assert!(matches!(
            output.process.read_config,
            ProcessingStepResult::Failed(_)
        ));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (auto_build calls build internally)
        let output = crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true, // ignore bare files
            None,
            true,
            false, // skip_gitignore
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());

        // Run build again (tests standalone rebuild)
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(
            !output.has_failed_step(),
            "build failed: {:#?}",
            output.process
        );
        assert!(output.result.is_some());

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
            .field_with_name("_embedding")
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

        // Run init (auto_build calls build internally)
        let output = crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false, // skip_gitignore
            false, // verbose
        )
        .await;
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
        let bad_batch = build_chunks_batch(&bad_chunks, 2, "_");
        write_parquet(&tmp.path().join(".mdvs/chunks.parquet"), &bad_batch).unwrap();

        // Add a new file so model gets loaded (incremental detects new file)
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\ntags:\n  - test\ndraft: true\n---\n# New\nNew content.",
        )
        .unwrap();

        // Build should fail with dimension mismatch when model loads
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(output.has_failed_step());
        let err = match &output.process.embed_files {
            ProcessingStepResult::Failed(e) => &e.message,
            other => panic!("expected embed_files Failed, got: {other:?}"),
        };
        assert!(err.contains("dimension mismatch"));
    }

    #[tokio::test]
    async fn missing_build_sections_filled() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init without auto-build (no build sections in toml)
        let output = crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true,
            None,
            false, // no auto_build
            false,
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());

        // Verify no build sections
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());
        assert!(config.search.is_none());

        // Build should fill defaults and succeed
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(
            !output.has_failed_step(),
            "build failed: {:#?}",
            output.process
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

        // Init with auto-build (sections exist)
        let output = crate::cmd::init::run(
            tmp.path(),
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
        .await;
        assert!(!output.has_failed_step());

        // Try to change model without --force
        let output = run(tmp.path(), Some("other-model"), None, None, false, false).await;
        assert!(output.has_failed_step());
        let err = match &output.process.scan {
            ProcessingStepResult::Failed(e) => &e.message,
            other => panic!("expected scan Failed, got: {other:?}"),
        };
        assert!(err.contains("--force"));
    }

    #[tokio::test]
    async fn set_chunk_size_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
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
        .await;
        assert!(!init_output.has_failed_step());

        let output = run(tmp.path(), None, None, Some(512), false, false).await;
        assert!(output.has_failed_step());
        let err = match &output.process.scan {
            ProcessingStepResult::Failed(e) => &e.message,
            other => panic!("expected scan Failed, got: {other:?}"),
        };
        assert!(err.contains("--force"));
    }

    #[tokio::test]
    async fn set_model_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
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
        .await;
        assert!(!init_output.has_failed_step());

        // Change chunk size with --force (same model so no dimension mismatch)
        let output = run(tmp.path(), None, None, Some(512), true, false).await;
        assert!(
            !output.has_failed_step(),
            "build with --force failed: {:#?}",
            output.process
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
        .await;
        assert!(!init_output.has_failed_step());

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(output.has_failed_step());
        let err = match &output.process.classify {
            ProcessingStepResult::Failed(e) => &e.message,
            other => panic!("expected classify Failed, got: {other:?}"),
        };
        assert!(err.contains("config changed since last build"));
        assert!(err.contains("chunk_size"));

        // Build with --force should succeed
        let output = run(tmp.path(), None, None, None, true, false).await;
        assert!(
            !output.has_failed_step(),
            "build with --force failed: {:#?}",
            output.process
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
            update: crate::schema::config::UpdateConfig { auto_build: false },
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
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(output.has_violations(), "expected validation violations");
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
            update: crate::schema::config::UpdateConfig { auto_build: false },
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
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(output.has_violations(), "expected validation violations");
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
            update: crate::schema::config::UpdateConfig { auto_build: false },
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
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build should succeed despite unknown "author" field
        let output = run(tmp.path(), None, None, None, false, false).await;
        assert!(
            !output.has_failed_step(),
            "build should succeed with new fields: {:#?}",
            output.process
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
        .await;
        assert!(!init_output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());

        // Build again with no changes
        let output = run(tmp.path(), None, None, None, false, false).await;
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
        .await;
        assert!(!init_output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());
        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: Third\ntags:\n  - new\ndraft: false\n---\n# Third\nNew post content.",
        )
        .unwrap();

        let output = run(tmp.path(), None, None, None, false, false).await;
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
        .await;
        assert!(!init_output.has_failed_step());

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

        let output = run(tmp.path(), None, None, None, false, false).await;
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
        .await;
        assert!(!init_output.has_failed_step());

        let (files_before, _) = read_index_state(tmp.path());
        assert_eq!(files_before.len(), 2);

        // Remove post2
        fs::remove_file(tmp.path().join("blog/post2.md")).unwrap();

        let output = run(tmp.path(), None, None, None, false, false).await;
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
        .await;
        assert!(!init_output.has_failed_step());

        let (_, chunks_before) = read_index_state(tmp.path());
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Change only frontmatter (add a tag), keep same body
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\n  - new-tag\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, false).await;
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
        .await;
        assert!(!init_output.has_failed_step());

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let old_file_ids: HashSet<String> = files_before.values().cloned().collect();
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Force rebuild — should generate all new IDs
        let output = run(tmp.path(), None, None, None, true, false).await;
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
}
