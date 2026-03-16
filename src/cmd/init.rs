use crate::output::{
    format_file_count, format_hints, format_json_compact, CommandOutput, DiscoveredField,
};
use crate::pipeline::infer::{run_infer, InferOutput};
use crate::pipeline::scan::{run_scan, ScanOutput};
use crate::pipeline::write_config::{run_write_config, WriteConfigOutput};
use crate::pipeline::{ErrorKind, ProcessingStepError, ProcessingStepResult};
use crate::schema::shared::ScanConfig;
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::{info, instrument};

// ============================================================================
// InitResult
// ============================================================================

/// Result of the `init` command: discovered fields from schema inference.
#[derive(Debug, Serialize)]
pub struct InitResult {
    /// Directory where `mdvs.toml` was written.
    pub path: PathBuf,
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Fields inferred from frontmatter.
    pub fields: Vec<DiscoveredField>,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
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
            out.push_str("(dry run, nothing written)\n");
        } else {
            out.push_str(&format!(
                "\nInitialized mdvs in '{}'\n",
                self.path.display()
            ));
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
            out
        }
    }
}

// ============================================================================
// run()
// ============================================================================

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
        },
        result: None,
    }
}

/// Scan a directory, infer frontmatter schema, and write `mdvs.toml`.
/// Schema-only — no model download, no embedding, no `.mdvs/` created.
#[instrument(name = "init", skip_all)]
pub fn run(
    path: &Path,
    glob: &str,
    force: bool,
    dry_run: bool,
    ignore_bare_files: bool,
    skip_gitignore: bool,
    verbose: bool,
) -> InitCommandOutput {
    info!(path = %path.display(), "initializing");

    // Pre-checks
    if !path.is_dir() {
        return fail_at_scan(format!("'{}' is not a directory", path.display()));
    }

    let config_path = path.join("mdvs.toml");
    let mdvs_dir = path.join(".mdvs");
    if !force && (config_path.exists() || mdvs_dir.exists()) {
        return fail_at_scan(format!(
            "mdvs is already initialized in '{}' (use --force to reinitialize)",
            path.display()
        ));
    }

    // --force: delete existing artifacts
    if force {
        if config_path.exists() {
            let _ = std::fs::remove_file(&config_path);
        }
        if mdvs_dir.exists() {
            let _ = std::fs::remove_dir_all(&mdvs_dir);
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
                },
                result: None,
            };
        }
    };

    // 2. infer
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
                },
                result: None,
            };
        }
    };

    let total_files = scanned.files.len();
    info!(fields = schema.fields.len(), "schema inferred");

    // Build InitResult from scan+infer data
    let init_result = InitResult {
        path: path.to_path_buf(),
        files_scanned: total_files,
        fields: schema
            .fields
            .iter()
            .map(|f| f.to_discovered(total_files, verbose))
            .collect(),
        dry_run,
    };

    // 3. write_config (Skipped if dry_run)
    let write_config_step = if dry_run {
        ProcessingStepResult::Skipped
    } else {
        let (step, _config) = run_write_config(path, &schema, scan_config);
        step
    };

    InitCommandOutput {
        process: InitProcessOutput {
            scan: scan_step,
            infer: infer_step,
            write_config: write_config_step,
        },
        result: Some(init_result),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_test_vault(root: &Path) {
        let blog_dir = root.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text.",
        )
        .unwrap();
    }

    #[test]
    fn init_basic() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!output.has_failed_step());
        assert!(output.result.is_some());

        let result = output.result.unwrap();
        assert_eq!(result.files_scanned, 2);
        assert!(!result.fields.is_empty());
        assert!(!result.dry_run);

        // mdvs.toml should exist
        assert!(tmp.path().join("mdvs.toml").exists());
        // .mdvs/ should NOT exist (no build)
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn init_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(tmp.path(), "**", false, true, false, true, false);
        assert!(!output.has_failed_step());
        assert!(output.result.is_some());
        assert!(output.result.unwrap().dry_run);

        // Nothing written
        assert!(!tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn init_refuses_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // First init
        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!output.has_failed_step());

        // Second init without --force
        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(output.has_failed_step());
    }

    #[test]
    fn init_force_reinitializes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // First init
        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!output.has_failed_step());

        // Second init with --force
        let output = run(tmp.path(), "**", true, false, false, true, false);
        assert!(!output.has_failed_step());
    }

    #[test]
    fn init_force_cleans_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Create a fake .mdvs/ directory
        fs::create_dir_all(tmp.path().join(".mdvs")).unwrap();
        fs::write(tmp.path().join(".mdvs/files.parquet"), "fake").unwrap();

        // Init without --force should fail (mdvs already initialized)
        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(output.has_failed_step());

        // Init with --force should succeed and clean .mdvs/
        let output = run(tmp.path(), "**", true, false, false, true, false);
        assert!(!output.has_failed_step());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn init_no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("empty")).unwrap();

        let output = run(tmp.path(), "empty/**", false, false, false, true, false);
        assert!(output.has_failed_step());
    }

    #[test]
    fn init_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        fs::write(&file, "hello").unwrap();

        let output = run(&file, "**", false, false, false, true, false);
        assert!(output.has_failed_step());
    }

    #[test]
    fn init_config_has_check_section() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!output.has_failed_step());

        // Read back the config and verify [check] section
        let config = crate::schema::config::MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.check.is_some());
        assert!(config.check.unwrap().auto_update);
        // No model/chunking sections (filled by first build)
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());
        // Auto-flag sections are present
        assert!(config.build.is_some());
        assert!(config.build.unwrap().auto_update);
        assert!(config.search.is_some());
        assert!(config.search.as_ref().unwrap().auto_build);
        assert!(config.search.unwrap().auto_update);
    }

    #[test]
    fn hints_for_special_char_field_names() {
        use crate::output::FieldHint;

        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(
            tmp.path().join("test.md"),
            "---\nauthor's_note: hello\ntitle: Test\n---\n# Test\nBody.",
        )
        .unwrap();

        let output = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!output.has_failed_step());

        let result = output.result.unwrap();
        let sq_field = result
            .fields
            .iter()
            .find(|f| f.name == "author's_note")
            .unwrap();
        assert!(sq_field.hints.contains(&FieldHint::EscapeSingleQuotes));

        // Normal fields should have no hints
        let title_field = result.fields.iter().find(|f| f.name == "title").unwrap();
        assert!(title_field.hints.is_empty());
    }
}
