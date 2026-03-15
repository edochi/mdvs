use crate::cmd::build::BuildResult;
use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{content_hash, BuildMetadata, FileRow};
use crate::output::{
    format_file_count, format_hints, format_json_compact, CommandOutput, DiscoveredField,
};
use crate::pipeline::classify::{run_classify, ClassifyOutput};
use crate::pipeline::embed::{run_embed_files, EmbedFilesOutput};
use crate::pipeline::infer::{run_infer, InferOutput};
use crate::pipeline::load_model::{run_load_model, LoadModelOutput};
use crate::pipeline::scan::{run_scan, ScanOutput};
use crate::pipeline::validate::{run_validate, ValidateOutput};
use crate::pipeline::write_config::{run_write_config, WriteConfigOutput};
use crate::pipeline::write_index::{run_write_index, WriteIndexOutput};
use crate::pipeline::{ErrorKind, ProcessingStepError, ProcessingStepResult};
use crate::schema::shared::ScanConfig;
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::{info, instrument};

// ============================================================================
// InitResult
// ============================================================================

/// Result of the `init` command: discovered fields and optional build output.
#[derive(Debug, Serialize)]
pub struct InitResult {
    /// Directory where `mdvs.toml` was written.
    pub path: PathBuf,
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Fields inferred from frontmatter.
    pub fields: Vec<DiscoveredField>,
    /// Whether a build was triggered after initialization.
    pub auto_build: bool,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
    /// Build result, if a build was triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_result: Option<BuildResult>,
}

impl CommandOutput for InitResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        let field_summary = if self.fields.is_empty() {
            "no fields found".to_string()
        } else {
            format!("{} field(s)", self.fields.len())
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        out.push_str(&format!(
            "Initialized {} — {field_summary}{dry_run_suffix}\n",
            format_file_count(self.files_scanned)
        ));

        if !self.fields.is_empty() {
            out.push('\n');
            if verbose {
                // Record tables per field
                for field in &self.fields {
                    let mut builder = Builder::default();
                    builder.push_record([
                        format!("\"{}\"", field.name),
                        field.field_type.clone(),
                        format!("{}/{}", field.files_found, field.total_files),
                    ]);
                    let mut detail_lines = Vec::new();
                    if let Some(ref req) = field.required {
                        if !req.is_empty() {
                            detail_lines.push("  required:".to_string());
                            for g in req {
                                detail_lines.push(format!("    - \"{g}\""));
                            }
                        }
                    }
                    if let Some(ref allowed) = field.allowed {
                        detail_lines.push("  allowed:".to_string());
                        for g in allowed {
                            detail_lines.push(format!("    - \"{g}\""));
                        }
                    }
                    if field.nullable {
                        detail_lines.push("  nullable: true".to_string());
                    }
                    if !field.hints.is_empty() {
                        detail_lines.push(format!("  hints: {}", format_hints(&field.hints)));
                    }
                    builder.push_record([detail_lines.join("\n"), String::new(), String::new()]);
                    let mut table = builder.build();
                    style_record(&mut table, 3);
                    out.push_str(&format!("{table}\n"));
                }
            } else {
                // Compact table
                let mut builder = Builder::default();
                for field in &self.fields {
                    let type_str = if field.nullable {
                        format!("{}?", field.field_type)
                    } else {
                        field.field_type.clone()
                    };
                    let mut row = vec![
                        format!("\"{}\"", field.name),
                        type_str,
                        format!("{}/{}", field.files_found, field.total_files),
                    ];
                    let hints_str = format_hints(&field.hints);
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

        if self.dry_run {
            if self.auto_build {
                out.push_str("\nWould build index with model 'minishlab/potion-base-8M'\n");
            }
            out.push_str("(dry run, nothing written)\n");
        } else {
            out.push_str(&format!(
                "\nInitialized mdvs in '{}'\n",
                self.path.display()
            ));
        }

        if let Some(ref br) = self.build_result {
            out.push('\n');
            out.push_str(&br.format_text(verbose));
        }

        out
    }
}

// ============================================================================
// InitCommandOutput (pipeline)
// ============================================================================

/// Step records for each phase of the init pipeline.
#[derive(Debug, Serialize)]
pub struct InitProcessOutput {
    /// Scan the project directory for markdown files.
    pub scan: ProcessingStepResult<ScanOutput>,
    /// Infer field types and glob patterns.
    pub infer: ProcessingStepResult<InferOutput>,
    /// Write `mdvs.toml` to disk.
    pub write_config: ProcessingStepResult<WriteConfigOutput>,
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

/// Complete output of the `init` command.
#[derive(Debug, Serialize)]
pub struct InitCommandOutput {
    /// Step-by-step process records.
    pub process: InitProcessOutput,
    /// Init result (present when init completes successfully).
    pub result: Option<InitResult>,
}

impl InitCommandOutput {
    /// Returns `true` if any step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.scan, ProcessingStepResult::Failed(_))
            || matches!(self.process.infer, ProcessingStepResult::Failed(_))
            || matches!(self.process.write_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.validate, ProcessingStepResult::Failed(_))
            || matches!(self.process.classify, ProcessingStepResult::Failed(_))
            || matches!(self.process.load_model, ProcessingStepResult::Failed(_))
            || matches!(self.process.embed_files, ProcessingStepResult::Failed(_))
            || matches!(self.process.write_index, ProcessingStepResult::Failed(_))
    }
}

impl CommandOutput for InitCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        if let Some(result) = &self.result {
            if verbose {
                let mut out = String::new();
                out.push_str(&format!("{}\n", self.process.scan.format_line("Scan")));
                out.push_str(&format!("{}\n", self.process.infer.format_line("Infer")));
                out.push_str(&format!(
                    "{}\n",
                    self.process.write_config.format_line("Write config")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.validate.format_line("Validate")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.classify.format_line("Classify")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.load_model.format_line("Load model")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.embed_files.format_line("Embed")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.write_index.format_line("Write index")
                ));
                out.push('\n');
                out.push_str(&result.format_text(verbose));
                out
            } else {
                result.format_text(verbose)
            }
        } else {
            // Pipeline didn't complete — show steps up to the failure
            let mut out = String::new();
            out.push_str(&format!("{}\n", self.process.scan.format_line("Scan")));
            if !matches!(self.process.infer, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.infer.format_line("Infer")));
            }
            if !matches!(self.process.write_config, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.write_config.format_line("Write config")
                ));
            }
            if !matches!(self.process.validate, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.validate.format_line("Validate")
                ));
            }
            if !matches!(self.process.classify, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.classify.format_line("Classify")
                ));
            }
            if !matches!(self.process.load_model, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.load_model.format_line("Load model")
                ));
            }
            if !matches!(self.process.embed_files, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.embed_files.format_line("Embed")
                ));
            }
            if !matches!(self.process.write_index, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.write_index.format_line("Write index")
                ));
            }
            out
        }
    }
}

// ============================================================================
// run()
// ============================================================================

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

/// Helper to construct a failed InitCommandOutput where failure lands on the scan step.
fn fail_at_scan(message: String) -> InitCommandOutput {
    InitCommandOutput {
        process: InitProcessOutput {
            scan: ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message,
            }),
            infer: ProcessingStepResult::Skipped,
            write_config: ProcessingStepResult::Skipped,
            validate: ProcessingStepResult::Skipped,
            classify: ProcessingStepResult::Skipped,
            load_model: ProcessingStepResult::Skipped,
            embed_files: ProcessingStepResult::Skipped,
            write_index: ProcessingStepResult::Skipped,
        },
        result: None,
    }
}

/// Scan a directory, infer frontmatter schema, write `mdvs.toml`, and optionally build.
#[allow(clippy::too_many_arguments)]
#[instrument(name = "init", skip_all)]
pub async fn run(
    path: &Path,
    model: Option<&str>,
    revision: Option<&str>,
    glob: &str,
    force: bool,
    dry_run: bool,
    ignore_bare_files: bool,
    chunk_size: Option<usize>,
    auto_build: bool,
    skip_gitignore: bool,
    verbose: bool,
) -> InitCommandOutput {
    info!(path = %path.display(), "initializing");

    // Pre-checks (land on scan as Failed(User))
    if !path.is_dir() {
        return fail_at_scan(format!("'{}' is not a directory", path.display()));
    }

    let config_path = path.join("mdvs.toml");
    if !force && config_path.exists() {
        return fail_at_scan(format!(
            "mdvs.toml already exists in '{}' (use --force to overwrite)",
            path.display()
        ));
    }

    if !auto_build {
        if model.is_some() {
            return fail_at_scan("--model has no effect without --auto-build".to_string());
        }
        if revision.is_some() {
            return fail_at_scan("--revision has no effect without --auto-build".to_string());
        }
        if chunk_size.is_some() {
            return fail_at_scan("--chunk-size has no effect without --auto-build".to_string());
        }
    }

    // 1. scan
    let scan_config = ScanConfig {
        glob: glob.to_string(),
        include_bare_files: !ignore_bare_files,
        skip_gitignore,
    };
    let (scan_step, scanned) = run_scan(path, &scan_config);
    let scanned = match scanned {
        Some(s) => s,
        None => {
            return InitCommandOutput {
                process: InitProcessOutput {
                    scan: scan_step,
                    infer: ProcessingStepResult::Skipped,
                    write_config: ProcessingStepResult::Skipped,
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

    // 2. infer (no-files pre-check lands here)
    let (infer_step, schema) = if scanned.files.is_empty() {
        (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: format!("no markdown files found in '{}'", path.display()),
            }),
            None,
        )
    } else {
        run_infer(&scanned)
    };
    let schema = match schema {
        Some(s) => s,
        None => {
            return InitCommandOutput {
                process: InitProcessOutput {
                    scan: scan_step,
                    infer: infer_step,
                    write_config: ProcessingStepResult::Skipped,
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

    let total_files = scanned.files.len();
    info!(fields = schema.fields.len(), "schema inferred");

    // Build InitResult from scan+infer data
    let mut init_result = InitResult {
        path: path.to_path_buf(),
        files_scanned: total_files,
        fields: schema
            .fields
            .iter()
            .map(|f| f.to_discovered(total_files, verbose))
            .collect(),
        auto_build,
        dry_run,
        build_result: None,
    };

    // 3. write_config (Skipped if dry_run)
    let (write_config_step, config) = if dry_run {
        (ProcessingStepResult::Skipped, None)
    } else {
        let model_name = model.unwrap_or(DEFAULT_MODEL);
        let max_chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);
        run_write_config(
            path,
            &schema,
            scan_config,
            model_name,
            revision,
            max_chunk_size,
            auto_build,
        )
    };

    // If dry_run or write_config failed: skip build steps, return result
    if dry_run || config.is_none() {
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: ProcessingStepResult::Skipped,
                classify: ProcessingStepResult::Skipped,
                load_model: ProcessingStepResult::Skipped,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            // Result is available even when dry_run (shows fields etc.)
            result: if dry_run { Some(init_result) } else { None },
        };
    }

    let config = config.unwrap();

    // If !auto_build: skip build steps, return result
    if !auto_build {
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: ProcessingStepResult::Skipped,
                classify: ProcessingStepResult::Skipped,
                load_model: ProcessingStepResult::Skipped,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            result: Some(init_result),
        };
    }

    // === Build steps (4-8) — only when auto_build && !dry_run ===

    // 4. validate
    let (validate_step, validation_data) = run_validate(&scanned, &config, false);
    let validation_data = match validation_data {
        Some(d) => d,
        None => {
            return InitCommandOutput {
                process: InitProcessOutput {
                    scan: scan_step,
                    infer: infer_step,
                    write_config: write_config_step,
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
    // Violations after init are a bug — the config was just written from the same scan data
    if !violations.is_empty() {
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: ProcessingStepResult::Failed(ProcessingStepError {
                    kind: ErrorKind::Application,
                    message: "validation failed after init — this is a bug".to_string(),
                }),
                classify: ProcessingStepResult::Skipped,
                load_model: ProcessingStepResult::Skipped,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // Convert schema fields for write_index
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
            return InitCommandOutput {
                process: InitProcessOutput {
                    scan: scan_step,
                    infer: infer_step,
                    write_config: write_config_step,
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
    let backend = Backend::parquet(path);

    // 5. classify — always full rebuild (init is first-time)
    let (classify_step, classify_data) = run_classify(&scanned, &[], vec![], true);
    let classify_data = match classify_data {
        Some(d) => d,
        None => {
            return InitCommandOutput {
                process: InitProcessOutput {
                    scan: scan_step,
                    infer: infer_step,
                    write_config: write_config_step,
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

    // 6. load_model (skip if nothing to embed)
    let (load_model_step, embedder) = if needs_embedding {
        run_load_model(embedding)
    } else {
        (ProcessingStepResult::Skipped, None)
    };

    // If load_model failed, skip embed_files and write_index
    if needs_embedding && embedder.is_none() {
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: ProcessingStepResult::Skipped,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // 7. embed_files
    let max_chunk_size = chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    let (embed_files_step, embed_data) = if needs_embedding {
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
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: embed_files_step,
                write_index: ProcessingStepResult::Skipped,
            },
            result: None,
        };
    }

    // 8. write_index — assemble file_rows + chunk_rows
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

    let write_index_step = run_write_index(
        &backend,
        &schema_fields,
        &file_rows,
        &chunk_rows,
        build_meta,
    );

    if matches!(write_index_step, ProcessingStepResult::Failed(_)) {
        return InitCommandOutput {
            process: InitProcessOutput {
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
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

    let build_result = BuildResult {
        full_rebuild: true,
        files_total: file_rows.len(),
        files_embedded: classify_data.needs_embedding.len(),
        files_unchanged: file_rows.len() - classify_data.needs_embedding.len(),
        files_removed: 0,
        chunks_total,
        chunks_embedded,
        chunks_unchanged,
        chunks_removed: 0,
        new_fields,
        embedded_files: if verbose {
            Some(embedded_details)
        } else {
            None
        },
        removed_files: None,
    };

    init_result.build_result = Some(build_result);

    InitCommandOutput {
        process: InitProcessOutput {
            scan: scan_step,
            infer: infer_step,
            write_config: write_config_step,
            validate: validate_step,
            classify: classify_step,
            load_model: load_model_step,
            embed_files: embed_files_step,
            write_index: write_index_step,
        },
        result: Some(init_result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::MdvsToml;
    use std::fs;

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text.",
        )
        .unwrap();

        fs::write(dir.join("bare.md"), "# No frontmatter\nJust content.").unwrap();
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true, // dry_run
            true, // ignore_bare_files
            None,
            true,  // auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(result.dry_run);
        assert!(!tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn dry_run_result_fields() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true, // dry_run
            true, // ignore_bare_files
            None,
            false, // no auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert_eq!(result.files_scanned, 2); // bare.md excluded
        assert!(!result.fields.is_empty());
        assert!(result.dry_run);
        assert!(!result.auto_build);

        // Check field structure
        let title = result.fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title.field_type, "String");
        assert_eq!(title.files_found, 2);
        assert_eq!(title.total_files, 2);
    }

    #[tokio::test]
    async fn existing_config_no_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false, // no force
            true,
            true,
            None,
            true,
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.scan {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected scan step to fail"),
        };
        assert!(msg.contains("already exists"));
        assert!(msg.contains("--force"));
    }

    #[tokio::test]
    async fn existing_config_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        // force + dry_run: bypasses the existing-file check, skips build
        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            true, // force
            true, // dry_run
            true,
            None,
            true,
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(!output.has_failed_step());
    }

    #[tokio::test]
    async fn no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        // empty directory, no .md files

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true,
            true,
            None,
            true,
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.infer {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected infer step to fail"),
        };
        assert!(msg.contains("no markdown files"));
    }

    #[tokio::test]
    async fn flag_validation_model_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            Some("some-model"),
            None,
            "**",
            false,
            true,
            true,
            None,
            false, // no auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.scan {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected scan step to fail"),
        };
        assert!(msg.contains("--model has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn flag_validation_revision_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            None,
            Some("abc123"),
            "**",
            false,
            true,
            true,
            None,
            false,
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.scan {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected scan step to fail"),
        };
        assert!(msg.contains("--revision has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn flag_validation_chunk_size_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true,
            true,
            Some(512),
            false,
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.scan {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected scan step to fail"),
        };
        assert!(msg.contains("--chunk-size has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn no_auto_build_skips_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false, // not dry_run — writes config
            true,
            None,
            false, // no auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        // Config written, but no .mdvs/ directory
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join(".mdvs").exists());
        assert!(!result.auto_build);

        // Build steps should be Skipped
        assert!(matches!(
            output.process.validate,
            ProcessingStepResult::Skipped
        ));
        assert!(matches!(
            output.process.classify,
            ProcessingStepResult::Skipped
        ));
        assert!(matches!(
            output.process.load_model,
            ProcessingStepResult::Skipped
        ));
        assert!(matches!(
            output.process.embed_files,
            ProcessingStepResult::Skipped
        ));
        assert!(matches!(
            output.process.write_index,
            ProcessingStepResult::Skipped
        ));

        // Verify config has no build sections
        let toml_doc = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.chunking.is_none());
        assert!(toml_doc.search.is_none());
    }

    #[tokio::test]
    async fn hints_for_special_char_field_names() {
        use crate::output::FieldHint;

        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("notes");
        fs::create_dir_all(&dir).unwrap();

        fs::write(
            dir.join("note.md"),
            "---\nauthor's_note: hello\ntitle: world\n---\n# Note\nBody.",
        )
        .unwrap();

        let output = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true, // dry_run
            true,
            None,
            false, // no auto_build
            false,
            false,
        )
        .await;

        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        let sq_field = result
            .fields
            .iter()
            .find(|f| f.name == "author's_note")
            .expect("field with single quote should be discovered");
        assert!(sq_field.hints.contains(&FieldHint::EscapeSingleQuotes));

        // Normal fields should have no hints
        let title_field = result
            .fields
            .iter()
            .find(|f| f.name == "title")
            .expect("title field should be discovered");
        assert!(title_field.hints.is_empty());
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let model = "minishlab/potion-base-8M";
        let output = run(
            tmp.path(),
            Some(model),
            None,
            "**",
            false,
            false, // not dry_run — full pipeline
            true,  // ignore bare files
            None,
            true,  // auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;

        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(result.auto_build);
        assert!(!result.dry_run);

        // Verify files exist (build creates .mdvs/)
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(tmp.path().join(".mdvs").is_dir());

        // Verify config contents
        let toml_doc = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_doc.scan.glob, "**");
        assert!(!toml_doc.scan.include_bare_files);
        assert_eq!(toml_doc.embedding_model.as_ref().unwrap().name, model);
        assert!(!toml_doc.fields.field.is_empty());

        // Verify build result is present
        assert!(result.build_result.is_some());
    }
}
