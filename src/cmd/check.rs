use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::output::{
    format_file_count, format_json_compact, CommandOutput, FieldViolation, NewField, ViolatingFile,
    ViolationKind,
};
use crate::pipeline::read_config::ReadConfigOutput;
use crate::pipeline::scan::ScanOutput;
use crate::pipeline::validate::ValidateOutput;
use crate::pipeline::ProcessingStepResult;
use crate::schema::config::MdvsToml;
use crate::schema::shared::FieldTypeSerde;
use crate::table::{style_compact, style_record, Builder};
use globset::Glob;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, instrument};

/// Result of the `check` command: validation violations and unknown fields.
#[derive(Debug, Serialize)]
pub struct CheckResult {
    /// Number of markdown files checked.
    pub files_checked: usize,
    /// Schema violations (wrong type, missing required, disallowed).
    pub field_violations: Vec<FieldViolation>,
    /// Fields found in frontmatter but not defined in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
}

impl CheckResult {
    /// Returns `true` if any schema violations were found.
    pub fn has_violations(&self) -> bool {
        !self.field_violations.is_empty()
    }
}

impl CommandOutput for CheckResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        let violations_part = if self.has_violations() {
            format!("{} violation(s)", self.field_violations.len())
        } else {
            "no violations".to_string()
        };
        let new_fields_part = if self.new_fields.is_empty() {
            String::new()
        } else {
            format!(", {} new field(s)", self.new_fields.len())
        };
        out.push_str(&format!(
            "Checked {} files — {violations_part}{new_fields_part}\n",
            self.files_checked
        ));

        // Violations table
        if self.has_violations() {
            out.push('\n');
            if verbose {
                for v in &self.field_violations {
                    let mut builder = Builder::default();
                    let kind_str = match v.kind {
                        ViolationKind::MissingRequired => "MissingRequired",
                        ViolationKind::WrongType => "WrongType",
                        ViolationKind::Disallowed => "Disallowed",
                        ViolationKind::NullNotAllowed => "NullNotAllowed",
                    };
                    builder.push_record([
                        format!("\"{}\"", v.field),
                        kind_str.to_string(),
                        format_file_count(v.files.len()),
                    ]);
                    let detail: String = v
                        .files
                        .iter()
                        .map(|f| match &f.detail {
                            Some(d) => format!("  - \"{}\" ({d})", f.path.display()),
                            None => format!("  - \"{}\"", f.path.display()),
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    builder.push_record([detail, String::new(), String::new()]);
                    let mut table = builder.build();
                    style_record(&mut table, 3);
                    out.push_str(&format!("{table}\n"));
                }
            } else {
                let mut builder = Builder::default();
                for v in &self.field_violations {
                    let kind_str = match v.kind {
                        ViolationKind::MissingRequired => "MissingRequired",
                        ViolationKind::WrongType => "WrongType",
                        ViolationKind::Disallowed => "Disallowed",
                        ViolationKind::NullNotAllowed => "NullNotAllowed",
                    };
                    builder.push_record([
                        format!("\"{}\"", v.field),
                        kind_str.to_string(),
                        format_file_count(v.files.len()),
                    ]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
        }

        // New fields table
        if !self.new_fields.is_empty() {
            out.push('\n');
            if verbose {
                for nf in &self.new_fields {
                    let mut builder = Builder::default();
                    builder.push_record([
                        format!("\"{}\"", nf.name),
                        "new".to_string(),
                        format_file_count(nf.files_found),
                    ]);
                    let detail = match &nf.files {
                        Some(files) => files
                            .iter()
                            .map(|p| format!("  - \"{}\"", p.display()))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        None => String::new(),
                    };
                    builder.push_record([detail, String::new(), String::new()]);
                    let mut table = builder.build();
                    style_record(&mut table, 3);
                    out.push_str(&format!("{table}\n"));
                }
            } else {
                let mut builder = Builder::default();
                for nf in &self.new_fields {
                    builder.push_record([
                        format!("\"{}\"", nf.name),
                        "new".to_string(),
                        format_file_count(nf.files_found),
                    ]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
        }

        out
    }
}

// ============================================================================
// CheckCommandOutput (pipeline-based)
// ============================================================================

/// Pipeline record for the check command.
#[derive(Debug, Serialize)]
pub struct CheckProcessOutput {
    /// Read config step result.
    pub read_config: ProcessingStepResult<ReadConfigOutput>,
    /// Scan step result.
    pub scan: ProcessingStepResult<ScanOutput>,
    /// Validate step result.
    pub validate: ProcessingStepResult<ValidateOutput>,
}

/// Full output of the check command: pipeline steps + command result.
#[derive(Debug, Serialize)]
pub struct CheckCommandOutput {
    /// Processing steps and their outcomes.
    pub process: CheckProcessOutput,
    /// Command result (None if pipeline didn't complete).
    pub result: Option<CheckResult>,
}

impl CheckCommandOutput {
    /// Returns `true` if any processing step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.read_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.scan, ProcessingStepResult::Failed(_))
            || matches!(self.process.validate, ProcessingStepResult::Failed(_))
    }
}

impl CommandOutput for CheckCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        // If pipeline completed successfully, delegate to CheckResult
        if let Some(result) = &self.result {
            if verbose {
                // Show step lines before the result
                let mut out = String::new();
                out.push_str(&format!("{}\n", self.process.read_config.format_line()));
                out.push_str(&format!("{}\n", self.process.scan.format_line()));
                out.push_str(&format!("{}\n", self.process.validate.format_line()));
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
            out
        }
    }
}

/// Read config, scan files, and validate frontmatter against the schema.
#[instrument(name = "check", skip_all)]
pub fn run(path: &Path, verbose: bool) -> CheckCommandOutput {
    use crate::pipeline::read_config::run_read_config;
    use crate::pipeline::scan::run_scan;
    use crate::pipeline::validate::run_validate;

    let (read_config_step, config) = run_read_config(path);

    let (scan_step, scanned) = match &config {
        Some(cfg) => run_scan(path, &cfg.scan),
        None => (ProcessingStepResult::Skipped, None),
    };

    let (validate_step, validation_data) = match (&scanned, &config) {
        (Some(scanned), Some(cfg)) => run_validate(scanned, cfg, verbose),
        _ => (ProcessingStepResult::Skipped, None),
    };

    // Build CheckResult from validation data (if completed)
    let result = validation_data.map(|(field_violations, new_fields)| {
        let files_checked = scanned.as_ref().map_or(0, |s| s.files.len());
        CheckResult {
            files_checked,
            field_violations,
            new_fields,
        }
    });

    CheckCommandOutput {
        process: CheckProcessOutput {
            read_config: read_config_step,
            scan: scan_step,
            validate: validate_step,
        },
        result,
    }
}

/// Accumulator key for grouping violations by field, kind, and rule.
#[derive(PartialEq, Eq, Hash)]
struct ViolationKey {
    field: String,
    kind: ViolationKind,
    rule: String,
}

/// Validate scanned files against the schema in `mdvs.toml`. Reusable core called by both `check` and `build`.
#[instrument(name = "validate", skip_all)]
pub fn validate(
    scanned: &ScannedFiles,
    config: &MdvsToml,
    verbose: bool,
) -> anyhow::Result<CheckResult> {
    info!(files = scanned.files.len(), "validating frontmatter");

    let field_map: HashMap<&str, _> = config
        .fields
        .field
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let ignore_set: HashSet<&str> = config.fields.ignore.iter().map(|s| s.as_str()).collect();

    let mut violations: HashMap<ViolationKey, Vec<ViolatingFile>> = HashMap::new();
    let mut new_field_paths: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    check_field_values(
        scanned,
        &field_map,
        &ignore_set,
        &mut violations,
        &mut new_field_paths,
    )?;
    check_required_fields(scanned, config, &mut violations);

    let field_violations = collect_violations(violations);
    let new_fields = collect_new_fields(new_field_paths, verbose);

    info!(violations = field_violations.len(), "validation complete");

    Ok(CheckResult {
        files_checked: scanned.files.len(),
        field_violations,
        new_fields,
    })
}

/// Check each file's frontmatter fields for type mismatches, disallowed locations, and new fields.
fn check_field_values(
    scanned: &ScannedFiles,
    field_map: &HashMap<&str, &crate::schema::config::TomlField>,
    ignore_set: &HashSet<&str>,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
    new_field_paths: &mut BTreeMap<String, Vec<PathBuf>>,
) -> anyhow::Result<()> {
    for file in &scanned.files {
        let file_path_str = file.path.display().to_string();

        let frontmatter = match &file.data {
            Some(Value::Object(map)) => Some(map),
            _ => None,
        };

        if let Some(map) = frontmatter {
            for (field_name, value) in map {
                if ignore_set.contains(field_name.as_str()) {
                    continue;
                }

                if value.is_null() {
                    continue;
                }

                if let Some(toml_field) = field_map.get(field_name.as_str()) {
                    let expected = FieldType::try_from(&toml_field.field_type)
                        .map_err(|e| anyhow::anyhow!("invalid type for '{}': {}", field_name, e))?;
                    if !type_matches(&expected, value) {
                        let key = ViolationKey {
                            field: field_name.clone(),
                            kind: ViolationKind::WrongType,
                            rule: format!("type {}", toml_field.field_type),
                        };
                        violations.entry(key).or_default().push(ViolatingFile {
                            path: file.path.clone(),
                            detail: Some(format!("got {}", actual_type_name(value))),
                        });
                    }

                    if !matches_any_glob(&toml_field.allowed, &file_path_str) {
                        let key = ViolationKey {
                            field: field_name.clone(),
                            kind: ViolationKind::Disallowed,
                            rule: format!("allowed in {:?}", toml_field.allowed),
                        };
                        violations.entry(key).or_default().push(ViolatingFile {
                            path: file.path.clone(),
                            detail: None,
                        });
                    }
                } else {
                    new_field_paths
                        .entry(field_name.clone())
                        .or_default()
                        .push(file.path.clone());
                }
            }
        }
    }
    Ok(())
}

/// Check that required fields are present in files matching their required glob patterns.
fn check_required_fields(
    scanned: &ScannedFiles,
    config: &MdvsToml,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
) {
    for toml_field in &config.fields.field {
        if toml_field.required.is_empty() {
            continue;
        }

        for file in &scanned.files {
            let file_path_str = file.path.display().to_string();

            if !matches_any_glob(&toml_field.required, &file_path_str) {
                continue;
            }

            let value = file
                .data
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|map| map.get(&toml_field.name));

            match value {
                None => {
                    // Key absent → MissingRequired
                    let key = ViolationKey {
                        field: toml_field.name.clone(),
                        kind: ViolationKind::MissingRequired,
                        rule: format!("required in {:?}", toml_field.required),
                    };
                    violations.entry(key).or_default().push(ViolatingFile {
                        path: file.path.clone(),
                        detail: None,
                    });
                }
                Some(v) if v.is_null() && !toml_field.nullable => {
                    // Key present but null on non-nullable field → NullNotAllowed
                    let key = ViolationKey {
                        field: toml_field.name.clone(),
                        kind: ViolationKind::NullNotAllowed,
                        rule: format!("not nullable, required in {:?}", toml_field.required),
                    };
                    violations.entry(key).or_default().push(ViolatingFile {
                        path: file.path.clone(),
                        detail: None,
                    });
                }
                _ => {
                    // Key present with value (or null on nullable field) → pass
                }
            }
        }
    }
}

/// Convert the violations accumulator into a sorted list of `FieldViolation`.
fn collect_violations(
    violations: HashMap<ViolationKey, Vec<ViolatingFile>>,
) -> Vec<FieldViolation> {
    let mut field_violations: Vec<FieldViolation> = violations
        .into_iter()
        .map(|(key, files)| FieldViolation {
            field: key.field,
            kind: key.kind,
            rule: key.rule,
            files,
        })
        .collect();
    field_violations.sort_by(|a, b| a.field.cmp(&b.field));
    field_violations
}

/// Convert the new fields accumulator into a list of `NewField`.
fn collect_new_fields(
    new_field_paths: BTreeMap<String, Vec<PathBuf>>,
    verbose: bool,
) -> Vec<NewField> {
    new_field_paths
        .into_iter()
        .map(|(name, paths)| {
            let files_found = paths.len();
            NewField {
                name,
                files_found,
                files: if verbose { Some(paths) } else { None },
            }
        })
        .collect()
}

fn type_matches(expected: &FieldType, value: &Value) -> bool {
    match (expected, value) {
        // String is the top type in the widening hierarchy — accepts any value
        (FieldType::String, _) => true,
        (FieldType::Boolean, Value::Bool(_)) => true,
        (FieldType::Integer, Value::Number(n)) => n.is_i64() || n.is_u64(),
        (FieldType::Float, Value::Number(_)) => true, // lenient: accepts integers
        (FieldType::Array(inner), Value::Array(arr)) => arr.iter().all(|v| type_matches(inner, v)),
        (FieldType::Object(_), Value::Object(_)) => true,
        _ => false,
    }
}

fn matches_any_glob(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| {
        Glob::new(p)
            .ok()
            .map(|g| g.compile_matcher())
            .is_some_and(|m| m.is_match(path))
    })
}

fn actual_type_name(value: &Value) -> String {
    FieldTypeSerde::from(&FieldType::from(value)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::ScanConfig;
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

    fn write_toml(dir: &Path, fields: Vec<TomlField>, ignore: Vec<String>) {
        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: false },
            fields: FieldsConfig {
                ignore,
                field: fields,
            },
            embedding_model: None,
            chunking: None,
            search: None,
            storage: None,
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn string_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
        }
    }

    fn bool_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
        }
    }

    #[test]
    fn clean_check() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(!result.has_violations());
        assert!(result.new_fields.is_empty());
        assert_eq!(result.files_checked, 2);
    }

    #[test]
    fn missing_required() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // tags is required in blog/**, but post2 doesn't have tags
        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(result.has_violations());
        let v = &result.field_violations[0];
        assert_eq!(v.field, "tags");
        assert!(matches!(v.kind, ViolationKind::MissingRequired));
        assert_eq!(v.files.len(), 1);
        assert_eq!(v.files[0].path.display().to_string(), "blog/post2.md");
    }

    #[test]
    fn wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // draft is declared as Boolean but file has string value
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![string_field("title"), bool_field("draft")],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(result.has_violations());
        let v = &result.field_violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got String"));
    }

    #[test]
    fn wrong_type_int_in_float_lenient() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        // rating is declared Float, file has integer 5 — should be OK
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nrating: 5\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "rating".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
            }],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(!result.has_violations());
    }

    #[test]
    fn disallowed_field() {
        let tmp = tempfile::tempdir().unwrap();
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();

        // draft is only allowed in blog/**, but appears in notes/
        fs::write(
            notes_dir.join("idea.md"),
            "---\ndraft: true\n---\n# Idea\nContent.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "draft".into(),
                field_type: FieldTypeSerde::Scalar("Boolean".into()),
                allowed: vec!["blog/**".into()],
                required: vec![],
                nullable: false,
            }],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(result.has_violations());
        let v = &result.field_violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::Disallowed));
        assert_eq!(v.files[0].path.display().to_string(), "notes/idea.md");
    }

    #[test]
    fn new_fields_informational() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Only declare title — tags and draft are new
        write_toml(tmp.path(), vec![string_field("title")], vec![]);

        let result = run(tmp.path(), false).result.unwrap();

        assert!(!result.has_violations());
        assert_eq!(result.new_fields.len(), 2);
        assert!(result.new_fields.iter().any(|f| f.name == "draft"));
        assert!(result.new_fields.iter().any(|f| f.name == "tags"));
    }

    #[test]
    fn string_top_type_accepts_any_value() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // String field with boolean, integer, array, and object values
        fs::write(
            blog_dir.join("bool.md"),
            "---\nfield: false\n---\n# Bool\nBody.",
        )
        .unwrap();
        fs::write(blog_dir.join("int.md"), "---\nfield: 42\n---\n# Int\nBody.").unwrap();
        fs::write(
            blog_dir.join("array.md"),
            "---\nfield:\n  - a\n  - b\n---\n# Array\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("object.md"),
            "---\nfield:\n  k: v\n---\n# Object\nBody.",
        )
        .unwrap();

        write_toml(tmp.path(), vec![string_field("field")], vec![]);

        let result = run(tmp.path(), false).result.unwrap();

        assert!(!result.has_violations());
        assert_eq!(result.files_checked, 4);
    }

    #[test]
    fn bare_files_trigger_required() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        // A bare file (no frontmatter) in blog/ — should violate required
        fs::write(
            tmp.path().join("blog/bare.md"),
            "# No frontmatter\nJust content.",
        )
        .unwrap();

        // title required in blog/** with include_bare_files=true
        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: true,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: false },
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                }],
            },
            embedding_model: None,
            chunking: None,
            search: None,
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), false).result.unwrap();

        // Bare files missing required fields are violations
        assert!(result.has_violations());
    }

    #[test]
    fn ignored_fields_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // draft is in the ignore list — should not be flagged as new or violating
        write_toml(
            tmp.path(),
            vec![string_field("title")],
            vec!["draft".into(), "tags".into()],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(!result.has_violations());
        assert!(result.new_fields.is_empty()); // draft and tags are ignored, not new
    }

    #[test]
    fn multiple_violations() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&blog_dir).unwrap();
        fs::create_dir_all(&notes_dir).unwrap();

        // post1: draft is string "yes" (wrong type for Boolean), missing tags
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Post\nBody.",
        )
        .unwrap();

        // note1: has draft (disallowed outside blog/)
        fs::write(
            notes_dir.join("note1.md"),
            "---\ntitle: Note\ndraft: true\n---\n# Note\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                    nullable: false,
                },
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                },
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                    nullable: false,
                },
            ],
            vec![],
        );

        let result = run(tmp.path(), false).result.unwrap();

        assert!(result.has_violations());
        // Should have: draft wrong type, tags missing required, draft disallowed
        assert!(result.field_violations.len() >= 3);
    }
}
