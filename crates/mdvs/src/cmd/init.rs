use crate::cmd::steps::{infer_step, scan_step};
use crate::outcome::commands::InitOutcome;
use crate::outcome::{Outcome, WriteConfigOutcome};
use crate::output::{DiscoveredField, OutputFormat};
use crate::schema::config::MdvsToml;
use crate::schema::json_schema::{canonical_to_dsl, validate_mdvs_schema};
use crate::schema::load::load_schema;
use crate::schema::shared::{FieldTypeSerde, FrontmatterFormat, ScanConfig};
use crate::step::{CommandResult, ErrorKind, StepEntry, elapsed_ms};
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

/// Switches for [`run`], mirroring the `mdvs init` flags.
#[derive(Debug, Default, Clone, Copy)]
pub struct InitOptions {
    /// Overwrite an existing `mdvs.toml` instead of refusing (`--force`).
    pub force: bool,
    /// Report what would be written without writing anything (`--dry-run`).
    pub dry_run: bool,
    /// How init scans the vault.
    pub scan: InitScanFlags,
}

/// How init scans the vault. Each flag is persisted to the generated
/// `[scan]` section.
#[derive(Debug, Default, Clone, Copy)]
pub struct InitScanFlags {
    /// Leave files without frontmatter out of the scan
    /// (`--ignore-bare-files`, persisted as `include_bare_files = false`).
    pub ignore_bare_files: bool,
    /// Do not read `.gitignore` patterns during the scan
    /// (`--skip-gitignore`, persisted as `skip_gitignore = true`).
    pub skip_gitignore: bool,
}

/// Scan a directory, infer frontmatter schema, and write `mdvs.toml`.
/// Schema-only — no model download, no embedding, no `.mdvs/` created.
///
/// When `schema` is `Some(path)`, scanning + inference are skipped: the
/// schema file is loaded, validated against the mdvs subset, translated to
/// DSL fields, and written directly. `glob` and `opts.scan` still configure
/// the resulting `[scan]` section.
///
/// **Flag persistence.** Any flag the user passes to `init` that maps to a
/// config field is persisted to the generated `mdvs.toml` — that includes
/// `glob`, `ignore_bare_files`, `skip_gitignore`, and `default_output_format`
/// (the global `--output` flag). Flags that don't have a config equivalent
/// (`--force`, `--dry-run`, `--from-jsonschema`, `--logs`)
/// remain one-shot modifiers. The rule: if you cared enough to pass a flag
/// to `init`, you almost certainly want it to be the project default — so
/// it ends up in the file. When the flag is absent the corresponding field
/// is left unset (no `default_output_format` line at all), letting the
/// global default win.
#[instrument(name = "init", skip_all)]
pub fn run(
    path: &Path,
    glob: &str,
    opts: InitOptions,
    schema: Option<&Path>,
    default_output_format: Option<OutputFormat>,
) -> CommandResult {
    let InitOptions {
        force,
        dry_run,
        scan: InitScanFlags {
            ignore_bare_files,
            skip_gitignore,
        },
    } = opts;
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
        frontmatter_format: FrontmatterFormat::Auto,
    };

    // Schema-driven init: skip scan + infer, load+validate+translate, write.
    if let Some(schema_path) = schema {
        let outcome = init_from_schema(
            path,
            scan_config,
            schema_path,
            dry_run,
            default_output_format,
            &mut steps,
        );
        return match outcome {
            Ok(outcome) => init_result(steps, outcome, start),
            Err(()) => CommandResult::failed_from_steps(steps, start),
        };
    }

    let Ok(scanned) = scan_step(path, &scan_config, &mut steps) else {
        return CommandResult::failed_from_steps(steps, start);
    };

    if scanned.files.is_empty() {
        let msg = format!("no markdown files found in '{}'", path.display());
        steps.push(StepEntry::err(ErrorKind::User, msg.clone(), 0));
        return CommandResult::failed(steps, ErrorKind::User, msg, start);
    }

    let schema = infer_step(&scanned, &mut steps);
    for field in &schema.fields {
        field.emit_inexact_widening_warning();
    }

    let total_files = scanned.files.len();
    info!(fields = schema.fields.len(), "schema inferred");

    // Build fields — always with full detail (verbose=true) since the full outcome carries all data
    let fields: Vec<DiscoveredField> = schema
        .fields
        .iter()
        .map(|f| f.to_discovered(total_files, true))
        .collect();

    write_config_step(
        &config_path,
        schema.fields.len(),
        dry_run,
        &mut steps,
        || {
            let mut toml_doc = MdvsToml::from_inferred(&schema, scan_config);
            toml_doc.default_output_format = default_output_format;
            toml_doc
        },
    );

    let outcome = InitOutcome {
        path: path.to_path_buf(),
        files_scanned: total_files,
        fields,
        dry_run,
    };
    init_result(steps, outcome, start)
}

/// Wrap a successful init outcome into the command result.
fn init_result(steps: Vec<StepEntry>, outcome: InitOutcome, start: Instant) -> CommandResult {
    CommandResult {
        steps,
        result: Ok(Outcome::Init(Box::new(outcome))),
        elapsed_ms: elapsed_ms(start),
    }
}

/// Write the generated config, or push a skipped step under `--dry-run`.
///
/// `build` constructs the config inside the timed step. A write failure is
/// recorded as a failed step but does not fail the init command.
fn write_config_step(
    config_path: &Path,
    fields_written: usize,
    dry_run: bool,
    steps: &mut Vec<StepEntry>,
    build: impl FnOnce() -> MdvsToml,
) {
    if dry_run {
        steps.push(StepEntry::skipped());
        return;
    }
    let write_start = Instant::now();
    let mut toml_doc = build();
    match toml_doc.write(config_path) {
        Ok(()) => {
            steps.push(StepEntry::ok(
                Outcome::WriteConfig(WriteConfigOutcome {
                    config_path: config_path.display().to_string(),
                    fields_written,
                }),
                elapsed_ms(write_start),
            ));
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(write_start),
            ));
        }
    }
}

/// Schema-driven init: load the schema, validate it against the mdvs subset,
/// translate to DSL fields, build the `MdvsToml`, write it.
///
/// On failure the error step is pushed and `Err(())` is returned.
fn init_from_schema(
    path: &Path,
    scan_config: ScanConfig,
    schema_path: &Path,
    dry_run: bool,
    default_output_format: Option<OutputFormat>,
    steps: &mut Vec<StepEntry>,
) -> Result<InitOutcome, ()> {
    let config_path = path.join("mdvs.toml");

    let canonical = match load_schema(schema_path) {
        Ok(v) => v,
        Err(e) => {
            steps.push(StepEntry::err(ErrorKind::User, e.to_string(), 0));
            return Err(());
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
        return Err(());
    }

    let import = match canonical_to_dsl(&canonical) {
        Ok(v) => v,
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                format!("cannot import schema '{}': {e}", schema_path.display()),
                0,
            ));
            return Err(());
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

    write_config_step(&config_path, total_fields, dry_run, steps, || {
        let mut toml_doc = MdvsToml::default_with_fields(import.fields, import.ignore);
        toml_doc.scan = scan_config;
        toml_doc.default_output_format = default_output_format;
        toml_doc
    });

    Ok(InitOutcome {
        path: path.to_path_buf(),
        files_scanned: 0,
        fields: fields_for_outcome,
        dry_run,
    })
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

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
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

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                dry_run: true,
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_init(&step);
        assert!(result.dry_run);
        assert!(!tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn init_refuses_without_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_reinitializes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                force: true,
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));
    }

    #[test]
    fn init_force_cleans_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        fs::create_dir_all(tmp.path().join(".mdvs")).unwrap();
        fs::write(tmp.path().join(".mdvs/files.parquet"), "fake").unwrap();

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(crate::step::has_failed(&step));

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                force: true,
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
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
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not-a-dir");
        fs::write(&file, "hello").unwrap();

        let step = run(
            &file,
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn init_config_has_check_section() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
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

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
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
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            None,
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
            InitOptions {
                dry_run: true,
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            None,
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
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            None,
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
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            None,
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
            InitOptions {
                force: true,
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            None,
        );
        assert!(!crate::step::has_failed(&step));
        let content = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert!(content.contains("name = \"x\""));
    }

    // --- default_output_format persistence ---

    #[test]
    fn init_with_explicit_output_flag_persists_default_output_format() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.md"), "---\nstatus: draft\n---\nhi").unwrap();

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            Some(OutputFormat::Markdown),
        );
        assert!(!crate::step::has_failed(&step));

        let toml_path = tmp.path().join("mdvs.toml");
        let content = fs::read_to_string(&toml_path).unwrap();
        assert!(
            content.contains("default_output_format = \"markdown\""),
            "expected default_output_format = \"markdown\" in mdvs.toml, got:\n{content}"
        );
    }

    #[test]
    fn init_without_output_flag_omits_default_output_format() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.md"), "---\nstatus: draft\n---\nhi").unwrap();

        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));

        let content = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert!(
            !content.contains("default_output_format"),
            "expected no default_output_format line, got:\n{content}"
        );
    }

    #[test]
    fn init_force_overwrites_existing_default_output_format() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.md"), "---\nstatus: draft\n---\nhi").unwrap();

        // First init persists markdown
        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            Some(OutputFormat::Markdown),
        );
        assert!(!crate::step::has_failed(&step));

        // Re-init with --force and a different --output → overwrites
        let step = run(
            tmp.path(),
            "**",
            InitOptions { force: true, dry_run: // force
            false, scan: InitScanFlags { skip_gitignore: true, ..Default::default() } },
            None,
            Some(OutputFormat::Json),
        );
        assert!(!crate::step::has_failed(&step));

        let content = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert!(content.contains("default_output_format = \"json\""));
        assert!(!content.contains("\"markdown\""));
    }

    #[test]
    fn init_from_schema_with_output_flag_persists_default_output_format() {
        let tmp = tempfile::tempdir().unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"type": "object", "properties": {"x": {"type": "string"}}, "additionalProperties": true}"#,
        );
        let step = run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    skip_gitignore: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            Some(&schema_path),
            Some(OutputFormat::Json),
        );
        assert!(!crate::step::has_failed(&step));

        let content = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert!(content.contains("default_output_format = \"json\""));
    }
}
