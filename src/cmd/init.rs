use crate::outcome::commands::InitOutcome;
use crate::outcome::{InferOutcome, Outcome, ScanOutcome, WriteConfigOutcome};
use crate::output::DiscoveredField;
use crate::schema::shared::ScanConfig;
use crate::step::{from_pipeline_result, ErrorKind, Step, StepError, StepOutcome};
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

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
    _verbose: bool,
) -> Step<Outcome> {
    use crate::pipeline::infer::run_infer;
    use crate::pipeline::scan::run_scan;
    use crate::pipeline::write_config::run_write_config;

    let start = Instant::now();
    let mut substeps = Vec::new();

    info!(path = %path.display(), "initializing");

    // Pre-checks
    if !path.is_dir() {
        return fail_early(
            substeps,
            start,
            ErrorKind::User,
            format!("'{}' is not a directory", path.display()),
            3, // scan + infer + write_config
        );
    }

    let config_path = path.join("mdvs.toml");
    let mdvs_dir = path.join(".mdvs");
    if !force && (config_path.exists() || mdvs_dir.exists()) {
        return fail_early(
            substeps,
            start,
            ErrorKind::User,
            format!(
                "mdvs is already initialized in '{}' (use --force to reinitialize)",
                path.display()
            ),
            3,
        );
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

    // 1. Scan
    let scan_config = ScanConfig {
        glob: glob.to_string(),
        include_bare_files: !ignore_bare_files,
        skip_gitignore,
    };
    let (scan_result, scanned) = run_scan(path, &scan_config);
    substeps.push(from_pipeline_result(scan_result, |o| {
        Outcome::Scan(ScanOutcome {
            files_found: o.files_found,
            glob: o.glob.clone(),
        })
    }));

    let scanned = match scanned {
        Some(s) => s,
        None => {
            return fail_from_last_substep(&mut substeps, start, 2); // infer + write_config
        }
    };

    // 2. Infer
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
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        }); // write_config
        return Step {
            substeps,
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: format!("no markdown files found in '{}'", path.display()),
                }),
                elapsed_ms: start.elapsed().as_millis() as u64,
            },
        };
    }

    let (infer_result, schema) = run_infer(&scanned);
    substeps.push(from_pipeline_result(infer_result, |o| {
        Outcome::Infer(InferOutcome {
            fields_inferred: o.fields_inferred,
        })
    }));

    let schema = match schema {
        Some(s) => s,
        None => {
            return fail_from_last_substep(&mut substeps, start, 1); // write_config
        }
    };

    let total_files = scanned.files.len();
    info!(fields = schema.fields.len(), "schema inferred");

    // Build fields — always with full detail (verbose=true) since the full outcome carries all data
    let fields: Vec<DiscoveredField> = schema
        .fields
        .iter()
        .map(|f| f.to_discovered(total_files, true))
        .collect();

    // 3. Write config (Skipped if dry_run)
    if dry_run {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    } else {
        let (write_result, _config) = run_write_config(path, &schema, scan_config);
        substeps.push(from_pipeline_result(write_result, |o| {
            Outcome::WriteConfig(WriteConfigOutcome {
                config_path: o.config_path.clone(),
                fields_written: o.fields_written,
            })
        }));
    }

    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Init(Box::new(InitOutcome {
                path: path.to_path_buf(),
                files_scanned: total_files,
                fields,
                dry_run,
            }))),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Helper: push N Skipped substeps and return a failed Step.
fn fail_early(
    mut substeps: Vec<Step<Outcome>>,
    start: Instant,
    kind: ErrorKind,
    message: String,
    skipped_count: usize,
) -> Step<Outcome> {
    for _ in 0..skipped_count {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    }
    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Err(StepError { kind, message }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

/// Helper: extract error from last substep, push N Skipped, return failed Step.
fn fail_from_last_substep(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    skipped_count: usize,
) -> Step<Outcome> {
    let msg = match substeps.last().map(|s| &s.outcome) {
        Some(StepOutcome::Complete { result: Err(e), .. }) => e.message.clone(),
        _ => "step failed".into(),
    };
    for _ in 0..skipped_count {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use crate::output::FieldHint;
    use crate::step::StepOutcome;
    use std::fs;

    fn unwrap_init(step: &Step<Outcome>) -> &InitOutcome {
        match &step.outcome {
            StepOutcome::Complete {
                result: Ok(Outcome::Init(o)),
                ..
            } => o,
            other => panic!("expected Ok(Init), got: {other:?}"),
        }
    }

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

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!crate::step::has_failed(&step));

        let result = unwrap_init(&step);
        assert_eq!(result.files_scanned, 2);
        assert!(!result.fields.is_empty());
        assert!(!result.dry_run);
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn init_dry_run() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, true, false, true, false);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_init(&step);
        assert!(result.dry_run);
        assert!(!tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn init_refuses_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_reinitializes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", true, false, false, true, false);
        assert!(!crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_cleans_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        fs::create_dir_all(tmp.path().join(".mdvs")).unwrap();
        fs::write(tmp.path().join(".mdvs/files.parquet"), "fake").unwrap();

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", true, false, false, true, false);
        assert!(!crate::step::has_failed(&step));
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn init_no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("empty")).unwrap();

        let step = run(tmp.path(), "empty/**", false, false, false, true, false);
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        fs::write(&file, "hello").unwrap();

        let step = run(&file, "**", false, false, false, true, false);
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_config_has_check_section() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!crate::step::has_failed(&step));

        let config = crate::schema::config::MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.check.is_some());
        assert!(config.check.unwrap().auto_update);
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());
        assert!(config.build.is_some());
        assert!(config.build.unwrap().auto_update);
        assert!(config.search.is_some());
        assert!(config.search.as_ref().unwrap().auto_build);
        assert!(config.search.unwrap().auto_update);
    }

    #[test]
    fn hints_for_special_char_field_names() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(
            tmp.path().join("test.md"),
            "---\nauthor's_note: hello\ntitle: Test\n---\n# Test\nBody.",
        )
        .unwrap();

        let step = run(tmp.path(), "**", false, false, false, true, false);
        assert!(!crate::step::has_failed(&step));

        let result = unwrap_init(&step);
        let sq_field = result
            .fields
            .iter()
            .find(|f| f.name == "author's_note")
            .unwrap();
        assert!(sq_field.hints.contains(&FieldHint::EscapeSingleQuotes));

        let title_field = result.fields.iter().find(|f| f.name == "title").unwrap();
        assert!(title_field.hints.is_empty());
    }
}
