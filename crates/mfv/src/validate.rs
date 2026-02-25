use mdvs_schema::{FieldType, Schema, infer_type, is_date_string};
use regex::Regex;
use serde_json::Value;

use crate::diagnostic::{Diagnostic, DiagnosticKind};
use crate::scan::ScannedFile;

/// Validate scanned files against a schema. Returns diagnostics for all violations.
pub fn validate(files: &[ScannedFile], schema: &Schema) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    for file in files {
        let rules = schema.rules_for_path(&file.rel_path);
        let fm = file.frontmatter.as_ref();

        for rule in &rules {
            let value = fm.and_then(|v| v.get(&rule.name));

            // 1. Required check
            if rule.is_required_at(&file.rel_path) && value.is_none() {
                diagnostics.push(Diagnostic {
                    file: file.rel_path.clone(),
                    field: rule.name.clone(),
                    kind: DiagnosticKind::MissingRequired,
                });
                continue;
            }

            let Some(value) = value else {
                continue;
            };

            // 2. Type check
            if !type_matches(&rule.field_type, value) {
                let got = infer_type(value);
                diagnostics.push(Diagnostic {
                    file: file.rel_path.clone(),
                    field: rule.name.clone(),
                    kind: DiagnosticKind::WrongType {
                        expected: rule.field_type.to_string(),
                        got: got.to_string(),
                    },
                });
                continue;
            }

            // 3. Pattern check (string/date fields)
            if let Some(pattern) = &rule.pattern
                && let Some(s) = value_as_string(value)
                && let Ok(re) = Regex::new(pattern)
                && !re.is_match(&s)
            {
                diagnostics.push(Diagnostic {
                    file: file.rel_path.clone(),
                    field: rule.name.clone(),
                    kind: DiagnosticKind::PatternMismatch {
                        pattern: pattern.clone(),
                        value: s,
                    },
                });
            }

            // 4. Enum values check
            if rule.field_type == FieldType::Enum
                && !rule.values.is_empty()
                && let Some(s) = value_as_string(value)
                && !rule.values.contains(&s)
            {
                diagnostics.push(Diagnostic {
                    file: file.rel_path.clone(),
                    field: rule.name.clone(),
                    kind: DiagnosticKind::InvalidEnum {
                        value: s,
                        allowed: rule.values.clone(),
                    },
                });
            }
        }

        // Allowed enforcement: fields listed in schema must be allowed at this path.
        // Fields NOT in the schema at all have no constraints (silently skipped).
        if let Some(Value::Object(map)) = fm {
            for key in map.keys() {
                if let Some(f) = schema.fields.iter().find(|f| f.name == *key)
                    && !f.is_allowed_at(&file.rel_path)
                {
                    diagnostics.push(Diagnostic {
                        file: file.rel_path.clone(),
                        field: key.clone(),
                        kind: DiagnosticKind::NotAllowed,
                    });
                }
            }
        }
    }

    diagnostics
}

/// Check if a JSON value matches the expected FieldType.
fn type_matches(expected: &FieldType, value: &Value) -> bool {
    match expected {
        FieldType::String => matches!(value, Value::String(_)),
        FieldType::StringArray => matches!(value, Value::Array(_)),
        FieldType::Date => {
            matches!(value, Value::String(s) if is_date_string(s))
        }
        FieldType::Boolean => matches!(value, Value::Bool(_)),
        FieldType::Integer => matches!(value, Value::Number(n) if n.is_i64() || n.is_u64()),
        FieldType::Float => matches!(value, Value::Number(_)),
        FieldType::Enum => matches!(value, Value::String(_)),
    }
}

/// Extract a string representation from a JSON value (for pattern/enum checks).
fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse_schema(s: &str) -> mdvs_schema::Schema {
        s.parse().unwrap()
    }

    fn make_file(rel_path: &str, fm: Value) -> ScannedFile {
        ScannedFile {
            rel_path: rel_path.to_string(),
            frontmatter: Some(fm),
        }
    }

    #[test]
    fn missing_required_field() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
        );

        let files = vec![make_file("test.md", json!({}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].kind, DiagnosticKind::MissingRequired);
    }

    #[test]
    fn wrong_type() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
"#,
        );

        let files = vec![make_file("test.md", json!({"title": 42}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert!(matches!(&diags[0].kind, DiagnosticKind::WrongType { .. }));
    }

    #[test]
    fn pattern_mismatch() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "doi"
type = "string"
pattern = "^10\\.\\d{4,9}/.*"
"#,
        );

        let files = vec![make_file("test.md", json!({"doi": "not-a-doi"}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert!(matches!(
            &diags[0].kind,
            DiagnosticKind::PatternMismatch { .. }
        ));
    }

    #[test]
    fn invalid_enum() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "status"
type = "enum"
values = ["draft", "published"]
"#,
        );

        let files = vec![make_file("test.md", json!({"status": "archived"}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert!(matches!(&diags[0].kind, DiagnosticKind::InvalidEnum { .. }));
    }

    #[test]
    fn valid_file_no_diagnostics() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
required = ["**"]

[[fields.field]]
name = "date"
type = "date"
"#,
        );

        let files = vec![make_file(
            "test.md",
            json!({"title": "Hello", "date": "2025-06-12"}),
        )];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn no_frontmatter_skips_non_required() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
"#,
        );

        let files = vec![ScannedFile {
            rel_path: "test.md".to_string(),
            frontmatter: None,
        }];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn no_frontmatter_required_field() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
required = ["**"]
"#,
        );

        let files = vec![ScannedFile {
            rel_path: "test.md".to_string(),
            frontmatter: None,
        }];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].kind, DiagnosticKind::MissingRequired);
    }

    #[test]
    fn path_scoped_rule() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "doi"
type = "string"
allowed = ["papers/**"]
required = ["papers/**"]
"#,
        );

        // File outside "papers/" — scoped rule should not apply
        let files = vec![make_file("blog/post.md", json!({}))];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty(), "scoped rule should not apply to blog/");

        // File inside "papers/" — rule applies, doi is missing
        let files = vec![make_file("papers/study.md", json!({}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].kind, DiagnosticKind::MissingRequired);
    }

    #[test]
    fn multiple_violations_same_file() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
required = ["**"]

[[fields.field]]
name = "date"
type = "date"
required = ["**"]
"#,
        );

        let files = vec![make_file("test.md", json!({}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 2);
        assert!(diags.iter().any(|d| d.field == "title"));
        assert!(diags.iter().any(|d| d.field == "date"));
    }

    #[test]
    fn integer_rejects_float() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "count"
type = "integer"
"#,
        );

        let files = vec![make_file("test.md", json!({"count": 3.14}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert!(
            matches!(&diags[0].kind, DiagnosticKind::WrongType { expected, .. } if expected == "integer")
        );
    }

    #[test]
    fn field_not_allowed_at_path() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "doi"
type = "string"
allowed = ["blog/**"]
"#,
        );

        let files = vec![make_file("notes/x.md", json!({"doi": "10.1234/test"}))];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].field, "doi");
        assert_eq!(diags[0].kind, DiagnosticKind::NotAllowed);
    }

    #[test]
    fn field_allowed_at_path_no_error() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "doi"
type = "string"
allowed = ["blog/**"]
"#,
        );

        let files = vec![make_file("blog/x.md", json!({"doi": "10.1234/test"}))];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn unlisted_field_no_constraint() {
        // Fields not in the schema have no constraints — no diagnostic produced
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
"#,
        );

        let files = vec![make_file("test.md", json!({"title": "Hi", "extra": "value"}))];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn allowed_everywhere_no_error() {
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
allowed = ["**"]
"#,
        );

        let files = vec![make_file("any/deep/path.md", json!({"title": "Hello"}))];
        let diags = validate(&files, &schema);
        assert!(diags.is_empty());
    }

    #[test]
    fn not_allowed_but_unlisted_field_passes() {
        // "title" is in schema with allowed = ["blog/**"] → not allowed at notes/
        // "extra" is NOT in schema → no constraints, passes
        let schema = parse_schema(
            r#"
[[fields.field]]
name = "title"
type = "string"
allowed = ["blog/**"]
"#,
        );

        let files = vec![make_file(
            "notes/x.md",
            json!({"title": "Hi", "extra": "value"}),
        )];
        let diags = validate(&files, &schema);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].field, "title");
        assert_eq!(diags[0].kind, DiagnosticKind::NotAllowed);
    }
}
