use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::outcome::commands::CheckOutcome;
use crate::outcome::{Outcome, ReadConfigOutcome, ScanOutcome, ValidateOutcome};
use crate::output::{FieldViolation, NewField, ViolatingFile, ViolationKind};
use crate::schema::config::MdvsToml;
use crate::schema::shared::FieldTypeSerde;
use crate::step::{from_pipeline_result, ErrorKind, Step, StepError, StepOutcome};
use globset::Glob;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, instrument};

// ============================================================================
// CheckResult — kept for build compatibility during migration
// ============================================================================

/// Result of validation. Used by both `check` and `build` commands.
/// Kept during migration; build still references this type.
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

// ============================================================================
// run()
// ============================================================================

/// Read config, optionally auto-update, scan files, and validate frontmatter.
#[instrument(name = "check", skip_all)]
pub async fn run(path: &Path, no_update: bool, verbose: bool) -> Step<Outcome> {
    use crate::pipeline::read_config::run_read_config;
    use crate::pipeline::scan::run_scan;

    let start = std::time::Instant::now();
    let mut substeps = Vec::new();

    // 1. Read config
    let (read_config_result, config) = run_read_config(path);
    substeps.push(from_pipeline_result(read_config_result, |o| {
        Outcome::ReadConfig(ReadConfigOutcome {
            config_path: o.config_path.clone(),
        })
    }));

    let config = match config {
        Some(c) => c,
        None => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            }); // scan
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            }); // validate
            let msg = match &substeps[0].outcome {
                StepOutcome::Complete { result: Err(e), .. } => e.message.clone(),
                _ => "failed to read config".into(),
            };
            return Step {
                substeps,
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::User,
                        message: msg,
                    }),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            };
        }
    };

    // 2. Auto-update (calls old update::run(), does not nest as substep during migration)
    let should_update = !no_update && config.check.as_ref().is_some_and(|c| c.auto_update);
    if should_update {
        let update_output = crate::cmd::update::run(path, &[], false, false, verbose).await;
        if update_output.has_failed_step() {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            }); // scan
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            }); // validate
            return Step {
                substeps,
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::User,
                        message: "auto-update failed".into(),
                    }),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            };
        }
    }

    // Re-read config if auto-update ran
    let config = if should_update {
        match MdvsToml::read(&path.join("mdvs.toml")) {
            Ok(c) => c,
            Err(e) => {
                substeps.push(Step {
                    substeps: vec![],
                    outcome: StepOutcome::Skipped,
                }); // scan
                substeps.push(Step {
                    substeps: vec![],
                    outcome: StepOutcome::Skipped,
                }); // validate
                return Step {
                    substeps,
                    outcome: StepOutcome::Complete {
                        result: Err(StepError {
                            kind: ErrorKind::Application,
                            message: format!("failed to re-read config: {e}"),
                        }),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    },
                };
            }
        }
    } else {
        config
    };

    // 3. Scan
    let (scan_result, scanned) = run_scan(path, &config.scan);
    substeps.push(from_pipeline_result(scan_result, |o| {
        Outcome::Scan(ScanOutcome {
            files_found: o.files_found,
            glob: o.glob.clone(),
        })
    }));

    let scanned = match scanned {
        Some(s) => s,
        None => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            }); // validate
            let msg = match &substeps.last().unwrap().outcome {
                StepOutcome::Complete { result: Err(e), .. } => e.message.clone(),
                _ => "scan failed".into(),
            };
            return Step {
                substeps,
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::Application,
                        message: msg,
                    }),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            };
        }
    };

    // 4. Validate
    let validate_start = std::time::Instant::now();
    let check_result = match validate(&scanned, &config, verbose) {
        Ok(r) => r,
        Err(e) => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::Application,
                        message: e.to_string(),
                    }),
                    elapsed_ms: validate_start.elapsed().as_millis() as u64,
                },
            });
            return Step {
                substeps,
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::Application,
                        message: "validation failed".into(),
                    }),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            };
        }
    };

    // Push validate substep
    substeps.push(Step {
        substeps: vec![],
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Validate(ValidateOutcome {
                files_checked: check_result.files_checked,
                violations: check_result.field_violations.clone(),
                new_fields: check_result.new_fields.clone(),
            })),
            elapsed_ms: validate_start.elapsed().as_millis() as u64,
        },
    });

    // Build command outcome
    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Check(Box::new(CheckOutcome {
                files_checked: check_result.files_checked,
                violations: check_result.field_violations,
                new_fields: check_result.new_fields,
            }))),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

// ============================================================================
// validate() — core validation logic, reused by build
// ============================================================================

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

                if let Some(toml_field) = field_map.get(field_name.as_str()) {
                    // Disallowed: field present at a path not in allowed
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

                    if value.is_null() {
                        // NullNotAllowed: null value on a non-nullable field
                        if !toml_field.nullable {
                            let key = ViolationKey {
                                field: field_name.clone(),
                                kind: ViolationKind::NullNotAllowed,
                                rule: "not nullable".to_string(),
                            };
                            violations.entry(key).or_default().push(ViolatingFile {
                                path: file.path.clone(),
                                detail: None,
                            });
                        }
                    } else {
                        // WrongType: value doesn't match declared type
                        let expected =
                            FieldType::try_from(&toml_field.field_type).map_err(|e| {
                                anyhow::anyhow!("invalid type for '{}': {}", field_name, e)
                            })?;
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
                _ => {}
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
        (FieldType::String, _) => true,
        (FieldType::Boolean, Value::Bool(_)) => true,
        (FieldType::Integer, Value::Number(n)) => n.is_i64() || n.is_u64(),
        (FieldType::Float, Value::Number(_)) => true,
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
    use crate::outcome::commands::CheckOutcome;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::ScanConfig;
    use std::fs;

    fn unwrap_check(step: &Step<Outcome>) -> &CheckOutcome {
        match &step.outcome {
            StepOutcome::Complete {
                result: Ok(Outcome::Check(o)),
                ..
            } => o,
            other => panic!("expected Ok(Check), got: {other:?}"),
        }
    }

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
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore,
                field: fields,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
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

    #[tokio::test]
    async fn clean_check() {
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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert!(result.new_fields.is_empty());
        assert_eq!(result.files_checked, 2);
    }

    #[tokio::test]
    async fn missing_required() {
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
                    required: vec!["blog/**".into()],
                    nullable: false,
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "tags");
        assert!(matches!(v.kind, ViolationKind::MissingRequired));
        assert_eq!(v.files.len(), 1);
        assert_eq!(v.files[0].path.display().to_string(), "blog/post2.md");
    }

    #[tokio::test]
    async fn wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got String"));
    }

    #[tokio::test]
    async fn wrong_type_int_in_float_lenient() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[tokio::test]
    async fn disallowed_field() {
        let tmp = tempfile::tempdir().unwrap();
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();

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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::Disallowed));
        assert_eq!(v.files[0].path.display().to_string(), "notes/idea.md");
    }

    #[tokio::test]
    async fn new_fields_informational() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        write_toml(tmp.path(), vec![string_field("title")], vec![]);

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert_eq!(result.new_fields.len(), 2);
        assert!(result.new_fields.iter().any(|f| f.name == "draft"));
        assert!(result.new_fields.iter().any(|f| f.name == "tags"));
    }

    #[tokio::test]
    async fn string_top_type_accepts_any_value() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert_eq!(result.files_checked, 4);
    }

    #[tokio::test]
    async fn bare_files_trigger_required() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/bare.md"),
            "# No frontmatter\nJust content.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: true,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
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
            build: None,
            search: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
    }

    #[tokio::test]
    async fn ignored_fields_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        write_toml(
            tmp.path(),
            vec![string_field("title")],
            vec!["draft".into(), "tags".into()],
        );

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert!(result.new_fields.is_empty());
    }

    #[tokio::test]
    async fn multiple_violations() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&blog_dir).unwrap();
        fs::create_dir_all(&notes_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Post\nBody.",
        )
        .unwrap();

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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        assert!(result.violations.len() >= 3);
    }

    #[tokio::test]
    async fn null_on_non_nullable_non_required_field() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\nstatus:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "status".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "status")
            .expect("expected NullNotAllowed for status");
        assert!(matches!(v.kind, ViolationKind::NullNotAllowed));
    }

    #[tokio::test]
    async fn null_on_nullable_non_required_field() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\nstatus:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "status".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: true,
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[tokio::test]
    async fn null_on_disallowed_path() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\ndraft:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                    nullable: true,
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "draft")
            .expect("expected Disallowed for draft");
        assert!(matches!(v.kind, ViolationKind::Disallowed));
    }

    #[tokio::test]
    async fn null_on_disallowed_path_and_not_nullable() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\ndraft:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
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

        let step = run(tmp.path(), true, false).await;
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());

        let has_disallowed = result
            .violations
            .iter()
            .any(|v| v.field == "draft" && matches!(v.kind, ViolationKind::Disallowed));
        let has_null_not_allowed = result
            .violations
            .iter()
            .any(|v| v.field == "draft" && matches!(v.kind, ViolationKind::NullNotAllowed));

        assert!(has_disallowed, "expected Disallowed for draft");
        assert!(has_null_not_allowed, "expected NullNotAllowed for draft");
    }
}
