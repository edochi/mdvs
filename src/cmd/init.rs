use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::outcome::commands::InitOutcome;
use crate::outcome::{InferOutcome, Outcome, ScanOutcome, WriteConfigOutcome};
use crate::output::DiscoveredField;
use crate::schema::config::MdvsToml;
use crate::schema::shared::ScanConfig;
use crate::step::{CommandResult, ErrorKind, StepEntry, StepError};
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
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    info!(path = %path.display(), "initializing");

    // Pre-checks
    if !path.is_dir() {
        return fail_early(
            steps,
            start,
            ErrorKind::User,
            format!("'{}' is not a directory", path.display()),
        );
    }

    let config_path = path.join("mdvs.toml");
    let mdvs_dir = path.join(".mdvs");
    if !force && (config_path.exists() || mdvs_dir.exists()) {
        return fail_early(
            steps,
            start,
            ErrorKind::User,
            format!(
                "mdvs is already initialized in '{}' (use --force to reinitialize)",
                path.display()
            ),
        );
    }

    if force {
        if config_path.exists() {
            let _ = std::fs::remove_file(&config_path);
        }
        if mdvs_dir.exists() {
            let _ = std::fs::remove_dir_all(&mdvs_dir);
        }
    }

    // 1. Scan — calls ScannedFiles::scan() directly
    let scan_config = ScanConfig {
        glob: glob.to_string(),
        include_bare_files: !ignore_bare_files,
        skip_gitignore,
    };
    let scan_start = Instant::now();
    let scanned = match ScannedFiles::scan(path, &scan_config) {
        Ok(s) => {
            steps.push(StepEntry::ok(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: scan_config.glob.clone(),
                }),
                scan_start.elapsed().as_millis() as u64,
            ));
            s
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                scan_start.elapsed().as_millis() as u64,
            ));
            return fail_from_last_substep(&mut steps, start);
        }
    };

    // 2. Infer
    if scanned.files.is_empty() {
        let msg = format!("no markdown files found in '{}'", path.display());
        steps.push(StepEntry::err(ErrorKind::User, msg.clone(), 0));
        return CommandResult {
            steps,
            result: Err(StepError {
                kind: ErrorKind::User,
                message: msg,
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        };
    }

    // 2b. Infer — InferredSchema::infer() is infallible
    let infer_start = Instant::now();
    let schema = InferredSchema::infer(&scanned);
    steps.push(StepEntry::ok(
        Outcome::Infer(InferOutcome {
            fields_inferred: schema.fields.len(),
        }),
        infer_start.elapsed().as_millis() as u64,
    ));

    let total_files = scanned.files.len();
    info!(fields = schema.fields.len(), "schema inferred");

    // Build fields — always with full detail (verbose=true) since the full outcome carries all data
    let fields: Vec<DiscoveredField> = schema
        .fields
        .iter()
        .map(|f| f.to_discovered(total_files, true))
        .collect();

    // 3. Write config — MdvsToml::from_inferred() + write() directly
    if dry_run {
        steps.push(StepEntry::skipped());
    } else {
        let write_start = Instant::now();
        let toml_doc = MdvsToml::from_inferred(&schema, scan_config);
        match toml_doc.write(&config_path) {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::WriteConfig(WriteConfigOutcome {
                        config_path: config_path.display().to_string(),
                        fields_written: schema.fields.len(),
                    }),
                    write_start.elapsed().as_millis() as u64,
                ));
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    e.to_string(),
                    write_start.elapsed().as_millis() as u64,
                ));
            }
        }
    }

    CommandResult {
        steps,
        result: Ok(Outcome::Init(Box::new(InitOutcome {
            path: path.to_path_buf(),
            files_scanned: total_files,
            fields,
            dry_run,
        }))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

/// Helper: return a failed CommandResult with the given error.
fn fail_early(
    steps: Vec<StepEntry>,
    start: Instant,
    kind: ErrorKind,
    message: String,
) -> CommandResult {
    CommandResult {
        steps,
        result: Err(StepError { kind, message }),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

/// Helper: extract error from last step and return a failed CommandResult.
fn fail_from_last_substep(steps: &mut Vec<StepEntry>, start: Instant) -> CommandResult {
    let msg = match steps.last() {
        Some(StepEntry::Failed(f)) => f.message.clone(),
        _ => "step failed".into(),
    };
    CommandResult {
        steps: std::mem::take(steps),
        result: Err(StepError {
            kind: ErrorKind::Application,
            message: msg,
        }),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use crate::output::FieldHint;
    use crate::step::CommandResult;
    use std::fs;

    fn unwrap_init(result: &CommandResult) -> &InitOutcome {
        match &result.result {
            Ok(Outcome::Init(o)) => o,
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
