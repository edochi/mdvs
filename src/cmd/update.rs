use crate::cmd::build::{detect_config_changes, BuildResult};
use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{content_hash, BuildMetadata, FileRow};
use crate::output::{
    format_file_count, format_hints, format_json_compact, ChangedField, CommandOutput,
    DiscoveredField, FieldChange, RemovedField,
};
use crate::pipeline::classify::{run_classify, ClassifyOutput};
use crate::pipeline::embed::{run_embed_files, EmbedFilesOutput};
use crate::pipeline::infer::{run_infer, InferOutput};
use crate::pipeline::load_model::{run_load_model, LoadModelOutput};
use crate::pipeline::read_config::{run_read_config, ReadConfigOutput};
use crate::pipeline::scan::{run_scan, ScanOutput};
use crate::pipeline::validate::{run_validate, ValidateOutput};
use crate::pipeline::write_config::WriteConfigOutput;
use crate::pipeline::write_index::{run_write_index, WriteIndexOutput};
use crate::pipeline::{ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult};
use crate::schema::config::TomlField;
use crate::schema::shared::FieldTypeSerde;
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

// ============================================================================
// UpdateResult
// ============================================================================

/// Result of the `update` command: field changes discovered by re-inference.
#[derive(Debug, Serialize)]
pub struct UpdateResult {
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Newly discovered fields not previously in `mdvs.toml`.
    pub added: Vec<DiscoveredField>,
    /// Fields whose type or glob constraints changed during re-inference.
    pub changed: Vec<ChangedField>,
    /// Fields that disappeared from all files during re-inference.
    pub removed: Vec<RemovedField>,
    /// Number of fields that remained identical.
    pub unchanged: usize,
    /// Whether a build was triggered after updating.
    pub auto_build: bool,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
    /// Build result, if a build was triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_result: Option<BuildResult>,
}

impl UpdateResult {
    fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.changed.is_empty() || !self.removed.is_empty()
    }
}

impl CommandOutput for UpdateResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        let total_changes = self.added.len() + self.changed.len() + self.removed.len();
        let summary = if total_changes == 0 {
            "no changes".to_string()
        } else {
            format!("{total_changes} field(s) changed")
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        out.push_str(&format!(
            "Scanned {} — {summary}{dry_run_suffix}\n",
            format_file_count(self.files_scanned)
        ));

        if !self.has_changes() {
            return out;
        }

        // Changes table
        out.push('\n');
        if verbose {
            // Record tables per field change
            for field in &self.added {
                let mut builder = Builder::default();
                builder.push_record([
                    format!("\"{}\"", field.name),
                    "added".to_string(),
                    field.field_type.clone(),
                ]);
                let mut detail_lines = Vec::new();
                if let Some(ref globs) = field.allowed {
                    detail_lines.push("  found in:".to_string());
                    for g in globs {
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
            for field in &self.changed {
                let mut builder = Builder::default();
                builder.push_record(["field", "aspect", "old", "new"]);
                for (i, change) in field.changes.iter().enumerate() {
                    let name_col = if i == 0 {
                        format!("\"{}\"", field.name)
                    } else {
                        String::new()
                    };
                    let (old, new) = change.format_old_new();
                    builder.push_record([name_col, change.label().to_string(), old, new]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
            for field in &self.removed {
                let mut builder = Builder::default();
                builder.push_record([
                    format!("\"{}\"", field.name),
                    "removed".to_string(),
                    String::new(),
                ]);
                let detail = match &field.allowed {
                    Some(globs) => {
                        let mut lines = vec!["  previously in:".to_string()];
                        for g in globs {
                            lines.push(format!("    - \"{g}\""));
                        }
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
            // Compact: separate tables per category to avoid empty trailing columns
            if !self.added.is_empty() {
                let mut builder = Builder::default();
                for field in &self.added {
                    let globs_summary = field
                        .allowed
                        .as_ref()
                        .map(|g| {
                            g.iter()
                                .map(|s| format!("\"{s}\""))
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    let type_str = if field.nullable {
                        format!("{}?", field.field_type)
                    } else {
                        field.field_type.clone()
                    };
                    let mut row = vec![
                        format!("\"{}\"", field.name),
                        "added".to_string(),
                        type_str,
                        globs_summary,
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
            if !self.changed.is_empty() {
                let mut builder = Builder::default();
                for field in &self.changed {
                    let aspects: Vec<&str> = field.changes.iter().map(FieldChange::label).collect();
                    builder.push_record([format!("\"{}\"", field.name), aspects.join(", ")]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
            if !self.removed.is_empty() {
                let mut builder = Builder::default();
                for field in &self.removed {
                    builder.push_record([format!("\"{}\"", field.name), "removed".to_string()]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
        }

        // Build result
        if let Some(ref br) = self.build_result {
            out.push('\n');
            out.push_str(&br.format_text(verbose));
        }

        out
    }
}

// ============================================================================
// UpdateCommandOutput (pipeline)
// ============================================================================

/// Step records for each phase of the update pipeline.
#[derive(Debug, Serialize)]
pub struct UpdateProcessOutput {
    /// Read and parse `mdvs.toml`.
    pub read_config: ProcessingStepResult<ReadConfigOutput>,
    /// Scan the project directory for markdown files.
    pub scan: ProcessingStepResult<ScanOutput>,
    /// Infer field types and glob patterns.
    pub infer: ProcessingStepResult<InferOutput>,
    /// Write updated `mdvs.toml` to disk.
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

/// Complete output of the `update` command.
#[derive(Debug, Serialize)]
pub struct UpdateCommandOutput {
    /// Step-by-step process records.
    pub process: UpdateProcessOutput,
    /// Update result (present when update completes successfully).
    pub result: Option<UpdateResult>,
}

impl UpdateCommandOutput {
    /// Returns `true` if any step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.read_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.scan, ProcessingStepResult::Failed(_))
            || matches!(self.process.infer, ProcessingStepResult::Failed(_))
            || matches!(self.process.write_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.validate, ProcessingStepResult::Failed(_))
            || matches!(self.process.classify, ProcessingStepResult::Failed(_))
            || matches!(self.process.load_model, ProcessingStepResult::Failed(_))
            || matches!(self.process.embed_files, ProcessingStepResult::Failed(_))
            || matches!(self.process.write_index, ProcessingStepResult::Failed(_))
    }
}

impl CommandOutput for UpdateCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        if let Some(result) = &self.result {
            if verbose {
                let mut out = String::new();
                out.push_str(&format!(
                    "{}\n",
                    self.process.read_config.format_line("Read config")
                ));
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
            out.push_str(&format!(
                "{}\n",
                self.process.read_config.format_line("Read config")
            ));
            if !matches!(self.process.scan, ProcessingStepResult::Skipped) {
                out.push_str(&format!("{}\n", self.process.scan.format_line("Scan")));
            }
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

/// Helper to build a skipped-everything output with a failed read_config step.
fn fail_at_read_config(message: String) -> UpdateCommandOutput {
    UpdateCommandOutput {
        process: UpdateProcessOutput {
            read_config: ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message,
            }),
            scan: ProcessingStepResult::Skipped,
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

/// All build step types bundled to avoid clippy `type_complexity` on tuple returns.
type BuildSteps = (
    ProcessingStepResult<ValidateOutput>,
    ProcessingStepResult<ClassifyOutput>,
    ProcessingStepResult<LoadModelOutput>,
    ProcessingStepResult<EmbedFilesOutput>,
    ProcessingStepResult<WriteIndexOutput>,
);

/// Helper to build a skipped-rest output from given step results.
fn all_build_steps_skipped() -> BuildSteps {
    (
        ProcessingStepResult::Skipped,
        ProcessingStepResult::Skipped,
        ProcessingStepResult::Skipped,
        ProcessingStepResult::Skipped,
        ProcessingStepResult::Skipped,
    )
}

/// Re-scan files, infer field changes, and update `mdvs.toml`.
#[instrument(name = "update", skip_all)]
pub async fn run(
    path: &Path,
    reinfer: &[String],
    reinfer_all: bool,
    build_flag: Option<bool>,
    dry_run: bool,
    verbose: bool,
) -> UpdateCommandOutput {
    // Pre-check: flag conflict (lands on read_config)
    if !reinfer.is_empty() && reinfer_all {
        return fail_at_read_config("cannot use --reinfer and --reinfer-all together".to_string());
    }

    // 1. read_config
    let (read_config_step, config) = run_read_config(path);
    let mut config = match config {
        Some(c) => c,
        None => {
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
                    scan: ProcessingStepResult::Skipped,
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

    // Pre-check: reinfer field names exist in config (lands on read_config)
    for name in reinfer {
        if !config.fields.field.iter().any(|f| f.name == *name) {
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: ProcessingStepResult::Failed(ProcessingStepError {
                        kind: ErrorKind::User,
                        message: format!("field '{name}' is not in mdvs.toml"),
                    }),
                    scan: ProcessingStepResult::Skipped,
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
    }

    // 2. scan
    let (scan_step, scanned) = run_scan(path, &config.scan);
    let scanned = match scanned {
        Some(s) => s,
        None => {
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
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

    // 3. infer
    let (infer_step, schema) = run_infer(&scanned);
    let schema = match schema {
        Some(s) => s,
        None => {
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
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

    // --- Field comparison logic (inline) ---
    let (protected, targets): (Vec<TomlField>, Vec<TomlField>) = if reinfer_all {
        (vec![], config.fields.field.drain(..).collect())
    } else if !reinfer.is_empty() {
        config
            .fields
            .field
            .drain(..)
            .partition(|f| !reinfer.contains(&f.name))
    } else {
        // Default mode: all existing are protected, no targets
        (config.fields.field.drain(..).collect(), vec![])
    };

    let old_fields: HashMap<&str, &TomlField> =
        targets.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut new_fields: Vec<TomlField> = protected.clone();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = protected.len();

    for inf in &schema.fields {
        if protected.iter().any(|f| f.name == inf.name) {
            continue;
        }
        if config.fields.ignore.contains(&inf.name) {
            continue;
        }

        let new_type = FieldTypeSerde::from(&inf.field_type);
        let toml_field = TomlField {
            name: inf.name.clone(),
            field_type: new_type.clone(),
            allowed: inf.allowed.clone(),
            required: inf.required.clone(),
            nullable: inf.nullable,
        };

        if let Some(old_field) = old_fields.get(inf.name.as_str()) {
            if **old_field == toml_field {
                unchanged += 1;
            } else {
                let mut changes = Vec::new();
                if old_field.field_type != toml_field.field_type {
                    changes.push(FieldChange::Type {
                        old: old_field.field_type.to_string(),
                        new: new_type.to_string(),
                    });
                }
                if old_field.allowed != toml_field.allowed {
                    changes.push(FieldChange::Allowed {
                        old: old_field.allowed.clone(),
                        new: toml_field.allowed.clone(),
                    });
                }
                if old_field.required != toml_field.required {
                    changes.push(FieldChange::Required {
                        old: old_field.required.clone(),
                        new: toml_field.required.clone(),
                    });
                }
                if old_field.nullable != toml_field.nullable {
                    changes.push(FieldChange::Nullable {
                        old: old_field.nullable,
                        new: toml_field.nullable,
                    });
                }
                changed.push(ChangedField {
                    name: inf.name.clone(),
                    changes,
                });
            }
            new_fields.push(toml_field);
        } else {
            added.push(inf.to_discovered(total_files, verbose));
            new_fields.push(toml_field);
        }
    }

    let mut removed: Vec<RemovedField> = old_fields
        .iter()
        .filter(|(name, _)| !schema.fields.iter().any(|f| f.name == **name))
        .map(|(name, old_field)| RemovedField {
            name: name.to_string(),
            allowed: if verbose {
                Some(old_field.allowed.clone())
            } else {
                None
            },
        })
        .collect();
    removed.sort_by(|a, b| a.name.cmp(&b.name));

    info!(
        added = added.len(),
        changed = changed.len(),
        removed = removed.len(),
        "update complete"
    );

    let should_build = build_flag.unwrap_or(config.update.auto_build);

    let mut update_result = UpdateResult {
        files_scanned: total_files,
        added,
        changed,
        removed,
        unchanged,
        auto_build: should_build && !dry_run,
        dry_run,
        build_result: None,
    };

    // 4. write_config (Skipped if dry_run or no changes)
    let write_config_step = if dry_run || !update_result.has_changes() {
        ProcessingStepResult::Skipped
    } else {
        let start = Instant::now();
        let config_path = path.join("mdvs.toml");

        config.fields.field = new_fields;
        if let Err(e) = config.write(&config_path) {
            let (validate_step, classify_step, load_model_step, embed_files_step, write_index_step) =
                all_build_steps_skipped();
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
                    scan: scan_step,
                    infer: infer_step,
                    write_config: ProcessingStepResult::Failed(ProcessingStepError {
                        kind: ErrorKind::Application,
                        message: e.to_string(),
                    }),
                    validate: validate_step,
                    classify: classify_step,
                    load_model: load_model_step,
                    embed_files: embed_files_step,
                    write_index: write_index_step,
                },
                result: None,
            };
        }

        ProcessingStepResult::Completed(ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: WriteConfigOutput {
                config_path: config_path.display().to_string(),
                fields_written: config.fields.field.len(),
            },
        })
    };

    // Steps 5-9: build (Skipped if !should_build or dry_run or no changes)
    let should_run_build = should_build
        && !dry_run
        && update_result.has_changes()
        && matches!(write_config_step, ProcessingStepResult::Completed(_));

    if !should_run_build {
        let (validate_step, classify_step, load_model_step, embed_files_step, write_index_step) =
            all_build_steps_skipped();
        return UpdateCommandOutput {
            process: UpdateProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: validate_step,
                classify: classify_step,
                load_model: load_model_step,
                embed_files: embed_files_step,
                write_index: write_index_step,
            },
            result: Some(update_result),
        };
    }

    // === Build steps (5-9) ===

    // 5. validate
    let (validate_step, validation_data) = run_validate(&scanned, &config, false);
    let validation_data = match validation_data {
        Some(d) => d,
        None => {
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
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

    let (violations, _new_fields) = validation_data;
    // Violations after update are a bug — the config was just written from the same scan data
    if !violations.is_empty() {
        return UpdateCommandOutput {
            process: UpdateProcessOutput {
                read_config: read_config_step,
                scan: scan_step,
                infer: infer_step,
                write_config: write_config_step,
                validate: ProcessingStepResult::Failed(ProcessingStepError {
                    kind: ErrorKind::Application,
                    message: "validation failed after update — this is a bug".to_string(),
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
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
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

    // Config change detection (pre-check for classify step)
    let config_change_error = detect_config_changes(&backend, embedding, chunking, &config, false);

    // 6. classify — incremental by default
    let full_rebuild = !backend.exists();

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
                    return UpdateCommandOutput {
                        process: UpdateProcessOutput {
                            read_config: read_config_step,
                            scan: scan_step,
                            infer: infer_step,
                            write_config: write_config_step,
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
                    return UpdateCommandOutput {
                        process: UpdateProcessOutput {
                            read_config: read_config_step,
                            scan: scan_step,
                            infer: infer_step,
                            write_config: write_config_step,
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
            return UpdateCommandOutput {
                process: UpdateProcessOutput {
                    read_config: read_config_step,
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

    // 7. load_model (skip if nothing to embed)
    let (load_model_step, embedder) = if needs_embedding {
        run_load_model(embedding)
    } else {
        (ProcessingStepResult::Skipped, None)
    };

    // If load_model failed, skip embed_files and write_index
    if needs_embedding && embedder.is_none() {
        return UpdateCommandOutput {
            process: UpdateProcessOutput {
                read_config: read_config_step,
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

    // Dimension check (pre-check for embed_files)
    // Skip on full rebuild — old index is being discarded entirely.
    let dim_error = if full_rebuild {
        None
    } else {
        match &embedder {
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
            None if needs_embedding => None, // load_model failed — handled above
            None => None,
        }
    };

    // 8. embed_files
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
        return UpdateCommandOutput {
            process: UpdateProcessOutput {
                read_config: read_config_step,
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

    // 9. write_index — assemble file_rows + chunk_rows
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
        return UpdateCommandOutput {
            process: UpdateProcessOutput {
                read_config: read_config_step,
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

    update_result.build_result = Some(BuildResult {
        full_rebuild,
        files_total: file_rows.len(),
        files_embedded: classify_data.needs_embedding.len(),
        files_unchanged: file_rows.len() - classify_data.needs_embedding.len(),
        files_removed: classify_data.removed_count,
        chunks_total,
        chunks_embedded,
        chunks_unchanged,
        chunks_removed: classify_data.chunks_removed,
        new_fields: vec![],
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
    });

    UpdateCommandOutput {
        process: UpdateProcessOutput {
            read_config: read_config_step,
            scan: scan_step,
            infer: infer_step,
            write_config: write_config_step,
            validate: validate_step,
            classify: classify_step,
            load_model: load_model_step,
            embed_files: embed_files_step,
            write_index: write_index_step,
        },
        result: Some(update_result),
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
    }

    async fn init_no_build(dir: &Path) {
        let output = crate::cmd::init::run(
            dir, None, None, "**", false, false, true, // ignore bare files
            None, false, // no auto_build
            false, // skip_gitignore
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());
    }

    #[tokio::test]
    async fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let output = run(tmp.path(), &[], false, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert!(!result.has_changes());
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.unchanged, 3); // title, tags, draft
    }

    #[tokio::test]
    async fn new_fields_discovered() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Add a file with a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let output = run(tmp.path(), &[], false, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].name, "author");
        assert_eq!(result.added[0].field_type, "String");
        assert_eq!(result.added[0].files_found, 1);
        assert_eq!(result.added[0].total_files, 3);
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.unchanged, 3); // title, tags, draft

        // Verify toml was updated
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "author"));
    }

    #[tokio::test]
    async fn reinfer_changes_type() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Replace files so tags becomes a string instead of array
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags: single-tag\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let output = run(
            tmp.path(),
            &["tags".to_string()],
            false,
            Some(false),
            false,
            false,
        )
        .await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "tags");
        assert!(result.changed[0].changes.iter().any(|c| matches!(
            c,
            FieldChange::Type { new, .. } if new == "String"
        )));
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
    }

    #[tokio::test]
    async fn reinfer_removes_disappeared() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Remove tags from all files
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let output = run(
            tmp.path(),
            &["tags".to_string()],
            false,
            Some(false),
            false,
            false,
        )
        .await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].name, "tags");
        assert!(result.changed.is_empty());
        assert!(result.added.is_empty());

        // Verify toml no longer has tags
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(!toml.fields.field.iter().any(|f| f.name == "tags"));
    }

    #[tokio::test]
    async fn reinfer_unknown_field_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            &["nonexistent".to_string()],
            false,
            Some(false),
            false,
            false,
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.read_config {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected read_config step to fail"),
        };
        assert!(msg.contains("field 'nonexistent' is not in mdvs.toml"));
    }

    #[tokio::test]
    async fn reinfer_and_reinfer_all_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            &["tags".to_string()],
            true, // reinfer_all
            Some(false),
            false,
            false,
        )
        .await;

        assert!(output.has_failed_step());
        let msg = match &output.process.read_config {
            ProcessingStepResult::Failed(err) => &err.message,
            _ => panic!("expected read_config step to fail"),
        };
        assert!(msg.contains("cannot use --reinfer and --reinfer-all together"));
    }

    #[tokio::test]
    async fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), &[], true, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        // All fields are re-inferred with same types → unchanged
        assert_eq!(result.unchanged, 3);
        assert!(result.added.is_empty());
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());

        // Non-field config is preserved
        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before.scan, toml_after.scan);
        assert_eq!(toml_before.update, toml_after.update);
        assert_eq!(toml_before.embedding_model, toml_after.embedding_model);
        assert_eq!(toml_before.chunking, toml_after.chunking);
        assert_eq!(toml_before.search, toml_after.search);
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let toml_before = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), &[], false, Some(false), true, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert!(result.dry_run);
        assert_eq!(result.added.len(), 1);

        // write_config should be Skipped
        assert!(matches!(
            output.process.write_config,
            ProcessingStepResult::Skipped
        ));

        // Toml unchanged
        let toml_after = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before, toml_after);
    }

    #[tokio::test]
    async fn build_override_false() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init with auto_build (writes config with auto_build=true)
        let output = crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true,
            None,
            false, // no auto_build for init
            false, // skip_gitignore
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());

        // Add a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        // --build false should skip build even if auto_build is true in toml
        let output = run(tmp.path(), &[], false, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert!(!result.auto_build);
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn reinfer_all_detects_glob_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // Two files with frontmatter + one bare file
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\n---\n# Hello\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\n---\n# World\nMore.",
        )
        .unwrap();
        fs::write(blog_dir.join("bare.md"), "# No frontmatter\nJust content.").unwrap();

        // Init with ignore_bare_files=true → title required=["**"]
        let output = crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true, // ignore bare files
            None,
            false, // no auto_build
            false,
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_before = toml_before
            .fields
            .field
            .iter()
            .find(|f| f.name == "title")
            .unwrap();
        assert_eq!(title_before.required, vec!["**"]);

        // Flip include_bare_files to true
        let mut config = toml_before;
        config.scan.include_bare_files = true;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Reinfer all — globs should change even though types don't
        let output = run(tmp.path(), &[], true, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(result.has_changes());
        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "title");

        // Toml rewritten with narrower required
        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_after = toml_after
            .fields
            .field
            .iter()
            .find(|f| f.name == "title")
            .unwrap();
        assert!(!title_after.required.contains(&"**".to_string()));
    }

    #[tokio::test]
    async fn disappearing_field_stays_in_default_mode() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Remove tags from all files
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        // Default mode: tags should stay in toml even though it disappeared
        let output = run(tmp.path(), &[], false, Some(false), false, false).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();

        assert!(!result.has_changes());

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "tags"));
    }
}
