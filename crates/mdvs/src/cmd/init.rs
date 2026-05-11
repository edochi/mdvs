use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::outcome::commands::InitOutcome;
use crate::outcome::{InferOutcome, Outcome, ScanOutcome, WriteConfigOutcome};
use crate::output::DiscoveredField;
use crate::schema::config::MdvsToml;
use crate::schema::json_schema::{canonical_to_dsl, validate_mdvs_schema};
use crate::schema::load::load_schema;
use crate::schema::shared::{FieldTypeSerde, ScanConfig};
use crate::step::{CommandResult, ErrorKind, StepEntry};
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

/// Scan a directory, infer frontmatter schema, and write `mdvs.toml`.
/// Schema-only — no model download, no embedding, no `.mdvs/` created.
///
/// When `schema` is `Some(path)`, scanning + inference are skipped: the
/// schema file is loaded, validated against the mdvs subset, translated to
/// DSL fields, and written directly. The `glob`, `ignore_bare_files`, and
/// `skip_gitignore` parameters still configure the resulting `[scan]` section.
#[instrument(name = "init", skip_all)]
#[allow(clippy::too_many_arguments)] // CLI surface; a struct would just defer the same fields
pub fn run(
    path: &Path,
    glob: &str,
    force: bool,
    dry_run: bool,
    ignore_bare_files: bool,
    skip_gitignore: bool,
    _verbose: bool,
    schema: Option<&Path>,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    info!(path = %path.display(), "initializing");

    // Pre-checks
    if !path.is_dir() {
        return CommandResult::failed(
            steps,
            ErrorKind::User,
            format!("'{}' is not a directory", path.display()),
            start,
        );
    }

    let config_path = path.join("mdvs.toml");
    let mdvs_dir = path.join(".mdvs");
    if !force && (config_path.exists() || mdvs_dir.exists()) {
        return CommandResult::failed(
            steps,
            ErrorKind::User,
            format!(
                "mdvs is already initialized in '{}' (use --force to reinitialize)",
                path.display()
            ),
            start,
        );
    }

    // `--force` deletes existing config + index, but only for a real write.
    // Under `--dry-run`, leave the filesystem untouched.
    if force && !dry_run {
        if config_path.exists() {
            let _ = std::fs::remove_file(&config_path);
        }
        if mdvs_dir.exists() {
            let _ = std::fs::remove_dir_all(&mdvs_dir);
        }
    }

    let scan_config = ScanConfig {
        glob: glob.to_string(),
        include_bare_files: !ignore_bare_files,
        skip_gitignore,
    };

    // Schema-driven init: skip scan + infer, load+validate+translate, write.
    if let Some(schema_path) = schema {
        return init_from_schema(
            path,
            &config_path,
            scan_config,
            schema_path,
            dry_run,
            steps,
            start,
        );
    }
    // 1. Scan — calls ScannedFiles::scan() directly
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
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    };

    // 2. Infer
    if scanned.files.is_empty() {
        let msg = format!("no markdown files found in '{}'", path.display());
        steps.push(StepEntry::err(ErrorKind::User, msg.clone(), 0));
        return CommandResult::failed(steps, ErrorKind::User, msg, start);
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
        let mut toml_doc = MdvsToml::from_inferred(&schema, scan_config);
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

/// Schema-driven init: load the schema, validate it against the mdvs subset,
/// translate to DSL fields, build the `MdvsToml`, write it.
fn init_from_schema(
    path: &Path,
    config_path: &Path,
    scan_config: ScanConfig,
    schema_path: &Path,
    dry_run: bool,
    mut steps: Vec<StepEntry>,
    start: Instant,
) -> CommandResult {
    let canonical = match load_schema(schema_path) {
        Ok(v) => v,
        Err(e) => {
            steps.push(StepEntry::err(ErrorKind::User, e.to_string(), 0));
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    if let Err(e) = validate_mdvs_schema(&canonical) {
        steps.push(StepEntry::err(
            ErrorKind::User,
            format!(
                "schema '{}' is not in the mdvs subset: {e}",
                schema_path.display()
            ),
            0,
        ));
        return CommandResult::failed_from_steps(steps, start);
    }

    let import = match canonical_to_dsl(&canonical) {
        Ok(v) => v,
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                format!("cannot import schema '{}': {e}", schema_path.display()),
                0,
            ));
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    let total_fields = import.fields.len();
    info!(
        fields = total_fields,
        ignore = import.ignore.len(),
        "schema imported"
    );

    let fields_for_outcome: Vec<DiscoveredField> = import
        .fields
        .iter()
        .map(|f| DiscoveredField {
            name: f.name.clone(),
            field_type: FieldTypeSerde::from(
                &crate::discover::field_type::FieldType::try_from(&f.field_type)
                    .unwrap_or(crate::discover::field_type::FieldType::String),
            )
            .to_string(),
            files_found: 0,
            total_files: 0,
            allowed: Some(f.allowed.clone()),
            required: Some(f.required.clone()),
            nullable: f.nullable,
            hints: crate::output::field_hints(&f.name),
        })
        .collect();

    if dry_run {
        steps.push(StepEntry::skipped());
    } else {
        let write_start = Instant::now();
        let mut toml_doc = MdvsToml::default_with_fields(import.fields, import.ignore);
        toml_doc.scan = scan_config;
        match toml_doc.write(config_path) {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::WriteConfig(WriteConfigOutcome {
                        config_path: config_path.display().to_string(),
                        fields_written: total_fields,
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
            files_scanned: 0,
            fields: fields_for_outcome,
            dry_run,
        }))),
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

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
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

        let step = run(tmp.path(), "**", false, true, false, true, false, None);
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_init(&step);
        assert!(result.dry_run);
        assert!(!tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn init_refuses_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
        assert!(!crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_reinitializes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
        assert!(!crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", true, false, false, true, false, None);
        assert!(!crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_cleans_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        fs::create_dir_all(tmp.path().join(".mdvs")).unwrap();
        fs::write(tmp.path().join(".mdvs/files.parquet"), "fake").unwrap();

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
        assert!(crate::step::has_failed(&step));

        let step = run(tmp.path(), "**", true, false, false, true, false, None);
        assert!(!crate::step::has_failed(&step));
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn init_no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("empty")).unwrap();

        let step = run(
            tmp.path(),
            "empty/**",
            false,
            false,
            false,
            true,
            false,
            None,
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        fs::write(&file, "hello").unwrap();

        let step = run(&file, "**", false, false, false, true, false, None);
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_config_has_check_section() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
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

        let step = run(tmp.path(), "**", false, false, false, true, false, None);
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

    // ------------------------------------------------------------------------
    // --schema (TODO-0149 step 10)
    // ------------------------------------------------------------------------

    fn write_schema(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("schema.json");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn init_with_schema_writes_canonical_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 3},
                    "rating": {"type": "integer", "minimum": 0, "maximum": 5}
                },
                "additionalProperties": true
            }"#,
        );
        let step = run(
            tmp.path(),
            "**",
            false,
            false,
            false,
            true,
            false,
            Some(&schema_path),
        );
        assert!(!crate::step::has_failed(&step), "step failed: {step:?}");
        let result = unwrap_init(&step);
        assert_eq!(result.files_scanned, 0);
        assert_eq!(result.fields.len(), 2);
        let toml_path = tmp.path().join("mdvs.toml");
        let content = fs::read_to_string(&toml_path).unwrap();
        assert!(content.contains("name = \"title\""));
        assert!(content.contains("name = \"rating\""));
        assert!(content.contains("min_length = 3"));
        assert!(content.contains("min = 0"));
        assert!(content.contains("max = 5"));
    }

    #[test]
    fn init_with_schema_dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"type": "object", "properties": {"title": {"type": "string"}}, "additionalProperties": true}"#,
        );
        let step = run(
            tmp.path(),
            "**",
            false,
            true,
            false,
            true,
            false,
            Some(&schema_path),
        );
        assert!(!crate::step::has_failed(&step));
        assert!(!tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn init_with_invalid_schema_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"oneOf": [{"type": "string"}, {"type": "integer"}]}"#,
        );
        let step = run(
            tmp.path(),
            "**",
            false,
            false,
            false,
            true,
            false,
            Some(&schema_path),
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_with_schema_refuses_existing_toml_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("mdvs.toml"), "# existing").unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": true}"#,
        );
        let step = run(
            tmp.path(),
            "**",
            false,
            false,
            false,
            true,
            false,
            Some(&schema_path),
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_with_schema_and_force_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("mdvs.toml"), "# old").unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": true}"#,
        );
        let step = run(
            tmp.path(),
            "**",
            true,
            false,
            false,
            true,
            false,
            Some(&schema_path),
        );
        assert!(!crate::step::has_failed(&step));
        let content = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert!(content.contains("name = \"x\""));
    }
}
