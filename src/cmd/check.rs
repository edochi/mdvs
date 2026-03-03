use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::output::{
    CommandOutput, FieldViolation, NewField, ViolatingFile, ViolationKind,
};
use crate::schema::config::MdvsToml;
use crate::schema::shared::FieldTypeSerde;
use globset::Glob;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use tracing::instrument;

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
    fn format_human(&self) -> String {
        let mut out = String::new();

        if !self.has_violations() && self.new_fields.is_empty() {
            out.push_str(&format!(
                "Checked {} files — no violations\n",
                self.files_checked
            ));
            return out;
        }

        if self.has_violations() {
            for violation in &self.field_violations {
                out.push_str(&format!("{}: {}\n", violation.field, violation.rule));
                let label = match violation.kind {
                    ViolationKind::MissingRequired => "missing",
                    ViolationKind::WrongType => "wrong type",
                    ViolationKind::Disallowed => "disallowed",
                };
                let file_parts: Vec<String> = violation
                    .files
                    .iter()
                    .map(|f| match &f.detail {
                        Some(d) => format!("{} ({d})", f.path.display()),
                        None => f.path.display().to_string(),
                    })
                    .collect();
                out.push_str(&format!("  {label}: {}\n", file_parts.join(", ")));
                out.push('\n');
            }

            out.push_str(&format!(
                "Checked {} files — {} field violation(s)\n",
                self.files_checked,
                self.field_violations.len(),
            ));
        } else {
            out.push_str(&format!(
                "Checked {} files — no violations\n",
                self.files_checked
            ));
        }

        if !self.new_fields.is_empty() {
            out.push_str("\nNew fields (not in mdvs.toml):\n");
            for nf in &self.new_fields {
                let word = if nf.files_found == 1 { "file" } else { "files" };
                out.push_str(&format!("  {} ({} {word})\n", nf.name, nf.files_found));
            }
            out.push_str("Run 'mdvs update' to incorporate new fields.\n");
        }

        out
    }
}

/// Read config and scan files, then validate frontmatter against the schema.
#[instrument(name = "check", skip_all)]
pub fn run(path: &Path) -> anyhow::Result<CheckResult> {
    let config = MdvsToml::read(&path.join("mdvs.toml"))?;
    let scanned = ScannedFiles::scan(path, &config.scan);
    validate(&scanned, &config)
}

/// Validate scanned files against the schema in `mdvs.toml`. Reusable core called by both `check` and `build`.
#[instrument(name = "validate", skip_all)]
pub fn validate(scanned: &ScannedFiles, config: &MdvsToml) -> anyhow::Result<CheckResult> {
    // Build lookups
    let field_map: HashMap<&str, _> = config
        .fields
        .field
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let ignore_set: HashSet<&str> = config.fields.ignore.iter().map(|s| s.as_str()).collect();

    // Accumulate violations keyed by (field_name, kind_tag, rule)
    // kind_tag is a string so it can be a HashMap key
    let mut violations: HashMap<(String, String, String), Vec<ViolatingFile>> = HashMap::new();
    let mut new_field_counts: BTreeMap<String, usize> = BTreeMap::new();

    // Check each file's frontmatter fields
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

                // Null values (bare YAML keys like `field:`) are treated as absent
                if value.is_null() {
                    continue;
                }

                if let Some(toml_field) = field_map.get(field_name.as_str()) {
                    // Check type
                    let expected = FieldType::try_from(&toml_field.field_type)
                        .map_err(|e| anyhow::anyhow!("invalid type for '{}': {}", field_name, e))?;
                    if !type_matches(&expected, value) {
                        let rule = format!("type {}", toml_field.field_type);
                        let key = (field_name.clone(), "WrongType".into(), rule);
                        violations.entry(key).or_default().push(ViolatingFile {
                            path: file.path.clone(),
                            detail: Some(format!("got {}", actual_type_name(value))),
                        });
                    }

                    // Check allowed globs
                    if !matches_any_glob(&toml_field.allowed, &file_path_str) {
                        let rule = format!("allowed in {:?}", toml_field.allowed);
                        let key = (field_name.clone(), "Disallowed".into(), rule);
                        violations.entry(key).or_default().push(ViolatingFile {
                            path: file.path.clone(),
                            detail: None,
                        });
                    }
                } else {
                    // New field
                    *new_field_counts.entry(field_name.clone()).or_insert(0) += 1;
                }
            }
        }
    }

    // Check required fields
    for toml_field in &config.fields.field {
        if toml_field.required.is_empty() {
            continue;
        }

        for file in &scanned.files {
            let file_path_str = file.path.display().to_string();

            if !matches_any_glob(&toml_field.required, &file_path_str) {
                continue;
            }

            let has_field = file.data
                .as_ref()
                .and_then(|v| v.as_object())
                .and_then(|map| map.get(&toml_field.name))
                .is_some_and(|v| !v.is_null());

            if !has_field {
                let rule = format!("required in {:?}", toml_field.required);
                let key = (toml_field.name.clone(), "MissingRequired".into(), rule);
                violations.entry(key).or_default().push(ViolatingFile {
                    path: file.path.clone(),
                    detail: None,
                });
            }
        }
    }

    // Convert violations map to sorted Vec<FieldViolation>
    let mut field_violations: Vec<FieldViolation> = violations
        .into_iter()
        .map(|((field, kind_tag, rule), files)| {
            let kind = match kind_tag.as_str() {
                "MissingRequired" => ViolationKind::MissingRequired,
                "WrongType" => ViolationKind::WrongType,
                "Disallowed" => ViolationKind::Disallowed,
                _ => unreachable!(),
            };
            FieldViolation {
                field,
                kind,
                rule,
                files,
            }
        })
        .collect();
    field_violations.sort_by(|a, b| a.field.cmp(&b.field));

    // Convert new fields
    let new_fields: Vec<NewField> = new_field_counts
        .into_iter()
        .map(|(name, files_found)| NewField { name, files_found })
        .collect();

    Ok(CheckResult {
        files_checked: scanned.files.len(),
        field_violations,
        new_fields,
    })
}

fn type_matches(expected: &FieldType, value: &Value) -> bool {
    match (expected, value) {
        // String is the top type in the widening hierarchy — accepts any value
        (FieldType::String, _) => true,
        (FieldType::Boolean, Value::Bool(_)) => true,
        (FieldType::Integer, Value::Number(n)) => n.is_i64() || n.is_u64(),
        (FieldType::Float, Value::Number(_)) => true, // lenient: accepts integers
        (FieldType::Array(inner), Value::Array(arr)) => {
            arr.iter().all(|v| type_matches(inner, v))
        }
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
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn string_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
        }
    }

    fn bool_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
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
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let result = run(tmp.path()).unwrap();

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
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let result = run(tmp.path()).unwrap();

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

        let result = run(tmp.path()).unwrap();

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
            }],
            vec![],
        );

        let result = run(tmp.path()).unwrap();

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
            }],
            vec![],
        );

        let result = run(tmp.path()).unwrap();

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

        let result = run(tmp.path()).unwrap();

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
        fs::write(
            blog_dir.join("int.md"),
            "---\nfield: 42\n---\n# Int\nBody.",
        )
        .unwrap();
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

        let result = run(tmp.path()).unwrap();

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
                }],
            },
            embedding_model: None,
            chunking: None,
            search: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path()).unwrap();

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

        let result = run(tmp.path()).unwrap();

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
                },
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                },
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                },
            ],
            vec![],
        );

        let result = run(tmp.path()).unwrap();

        assert!(result.has_violations());
        // Should have: draft wrong type, tags missing required, draft disallowed
        assert!(result.field_violations.len() >= 3);
    }
}
