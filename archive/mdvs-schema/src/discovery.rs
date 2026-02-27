use std::collections::HashMap;

use serde_json::Value;

use crate::FieldType;

/// Default date formats tried during inference (Y-M-D family, unambiguous).
/// Order matters: more specific (datetime) before less specific (date).
pub const DEFAULT_DATE_FORMATS: &[&str] = &[
    "%Y-%m-%dT%H:%M:%S",
    "%Y-%m-%d %H:%M:%S",
    "%Y-%m-%d",
    "%Y/%m/%d",
    "%Y.%m.%d",
];

/// A field discovered by scanning frontmatter across files.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field name as it appears in frontmatter.
    pub name: String,
    /// Inferred type based on the most common value type seen.
    pub field_type: FieldType,
    /// Relative paths of files containing this field.
    pub files: Vec<String>,
    /// Detected date format (only set when `field_type` is `Date`).
    pub date_format: Option<String>,
}

/// Try each format against a string, return the first that parses.
pub fn detect_date_format<'a>(s: &str, formats: &'a [&str]) -> Option<&'a str> {
    formats.iter().copied().find(|fmt| parse_date(s, fmt))
}

/// Strict date/datetime parsing against a chrono format string.
pub fn parse_date(s: &str, fmt: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, fmt).is_ok()
        || chrono::NaiveDateTime::parse_from_str(s, fmt).is_ok()
}

/// Check if a string looks like a date in any default format.
pub fn is_date_string(s: &str) -> bool {
    detect_date_format(s, DEFAULT_DATE_FORMATS).is_some()
}

/// Infer the FieldType for a JSON value using default date formats.
pub fn infer_type(value: &Value) -> FieldType {
    infer_type_with_formats(value, DEFAULT_DATE_FORMATS).0
}

/// Infer the FieldType and detected date format for a JSON value.
fn infer_type_with_formats<'a>(
    value: &Value,
    date_formats: &'a [&str],
) -> (FieldType, Option<&'a str>) {
    match value {
        Value::Bool(_) => (FieldType::Boolean, None),
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                (FieldType::Integer, None)
            } else {
                (FieldType::Float, None)
            }
        }
        Value::String(s) => {
            if let Some(fmt) = detect_date_format(s, date_formats) {
                (FieldType::Date, Some(fmt))
            } else {
                (FieldType::String, None)
            }
        }
        Value::Array(_) => (FieldType::StringArray, None),
        _ => (FieldType::String, None),
    }
}

/// Scan frontmatter values and discover fields with type and date format inference.
///
/// Takes `(relative_path, frontmatter)` pairs and a list of date formats to try.
/// Tracks which files contain each field and infers the best date format per field.
pub fn discover_fields(
    file_frontmatters: &[(&str, Option<&Value>)],
    date_formats: &[&str],
) -> Vec<FieldInfo> {
    let mut field_types: HashMap<String, HashMap<FieldType, usize>> = HashMap::new();
    let mut field_date_formats: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut field_files: HashMap<String, Vec<String>> = HashMap::new();

    for (path, fm) in file_frontmatters {
        let Some(Value::Object(map)) = fm else {
            continue;
        };
        for (key, val) in map {
            let (ft, detected_fmt) = infer_type_with_formats(val, date_formats);
            *field_types
                .entry(key.clone())
                .or_default()
                .entry(ft)
                .or_insert(0) += 1;
            if let Some(fmt) = detected_fmt {
                *field_date_formats
                    .entry(key.clone())
                    .or_default()
                    .entry(fmt.to_string())
                    .or_insert(0) += 1;
            }
            field_files
                .entry(key.clone())
                .or_default()
                .push(path.to_string());
        }
    }

    let mut fields: Vec<FieldInfo> = field_types
        .into_iter()
        .map(|(name, type_counts)| {
            let (field_type, _) = type_counts.iter().max_by_key(|(_, count)| *count).unwrap();
            let files = field_files.remove(&name).unwrap_or_default();

            let date_format = if *field_type == FieldType::Date {
                field_date_formats
                    .get(&name)
                    .and_then(|fmts| fmts.iter().max_by_key(|(_, count)| *count))
                    .map(|(fmt, _)| fmt.clone())
            } else {
                None
            };

            FieldInfo {
                name,
                field_type: field_type.clone(),
                files,
                date_format,
            }
        })
        .collect();

    fields.sort_by(|a, b| b.files.len().cmp(&a.files.len()).then(a.name.cmp(&b.name)));
    fields
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn infer_types() {
        assert_eq!(infer_type(&json!(true)), FieldType::Boolean);
        assert_eq!(infer_type(&json!(42)), FieldType::Integer);
        assert_eq!(infer_type(&json!(3.14)), FieldType::Float);
        assert_eq!(infer_type(&json!("hello")), FieldType::String);
        assert_eq!(infer_type(&json!("2025-06-12")), FieldType::Date);
        assert_eq!(infer_type(&json!(["a", "b"])), FieldType::StringArray);
    }

    #[test]
    fn date_validation() {
        assert!(is_date_string("2025-06-12"));
        assert!(is_date_string("2025-06-12T10:00:00"));
        assert!(is_date_string("2025/06/12"));
        assert!(is_date_string("2025.06.12"));
        assert!(!is_date_string("not-a-date"));
        assert!(!is_date_string("2025-13-01")); // invalid month
    }

    #[test]
    fn detect_format_iso_date() {
        assert_eq!(
            detect_date_format("2025-06-12", DEFAULT_DATE_FORMATS),
            Some("%Y-%m-%d")
        );
    }

    #[test]
    fn detect_format_iso_datetime() {
        assert_eq!(
            detect_date_format("2025-06-12T10:00:00", DEFAULT_DATE_FORMATS),
            Some("%Y-%m-%dT%H:%M:%S")
        );
    }

    #[test]
    fn detect_format_slash() {
        assert_eq!(
            detect_date_format("2025/06/12", DEFAULT_DATE_FORMATS),
            Some("%Y/%m/%d")
        );
    }

    #[test]
    fn detect_format_dot() {
        assert_eq!(
            detect_date_format("2025.06.12", DEFAULT_DATE_FORMATS),
            Some("%Y.%m.%d")
        );
    }

    #[test]
    fn detect_format_custom() {
        let formats = &["%d/%m/%Y", "%Y-%m-%d"];
        assert_eq!(detect_date_format("31/12/2025", formats), Some("%d/%m/%Y"));
        // Ambiguous value: first format wins
        assert_eq!(detect_date_format("01/02/2025", formats), Some("%d/%m/%Y"));
    }

    #[test]
    fn detect_format_no_match() {
        assert_eq!(
            detect_date_format("not-a-date", DEFAULT_DATE_FORMATS),
            None
        );
    }

    #[test]
    fn parse_date_strict() {
        assert!(parse_date("2025-06-12", "%Y-%m-%d"));
        assert!(parse_date("2025-06-12T10:00:00", "%Y-%m-%dT%H:%M:%S"));
        assert!(!parse_date("2025-06-12T10:00:00", "%Y-%m-%d"));
        assert!(!parse_date("not-a-date", "%Y-%m-%d"));
    }

    #[test]
    fn discover_with_file_paths() {
        let fm1 = json!({"title": "A", "tags": ["x"], "date": "2025-01-01"});
        let fm2 = json!({"title": "B", "date": "2025-01-02"});
        let fm3 = json!({"title": "C", "author": "me"});

        let inputs: Vec<(&str, Option<&serde_json::Value>)> = vec![
            ("blog/a.md", Some(&fm1)),
            ("blog/b.md", Some(&fm2)),
            ("notes/c.md", Some(&fm3)),
        ];
        let fields = discover_fields(&inputs, DEFAULT_DATE_FORMATS);

        // title appears in all 3
        let title = fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title.files.len(), 3);
        assert!(title.files.contains(&"blog/a.md".to_string()));

        // date appears in 2, detected as ISO format
        let date = fields.iter().find(|f| f.name == "date").unwrap();
        assert_eq!(date.files.len(), 2);
        assert_eq!(date.field_type, FieldType::Date);
        assert_eq!(date.date_format, Some("%Y-%m-%d".to_string()));

        // author appears in 1
        let author = fields.iter().find(|f| f.name == "author").unwrap();
        assert_eq!(author.files.len(), 1);
        assert_eq!(author.files[0], "notes/c.md");
        assert_eq!(author.date_format, None);
    }

    #[test]
    fn discover_custom_date_format() {
        let fm1 = json!({"date": "31/12/2025"});
        let fm2 = json!({"date": "15/06/2024"});

        let inputs: Vec<(&str, Option<&serde_json::Value>)> = vec![
            ("a.md", Some(&fm1)),
            ("b.md", Some(&fm2)),
        ];
        let formats = &["%d/%m/%Y", "%Y-%m-%d"];
        let fields = discover_fields(&inputs, formats);

        let date = fields.iter().find(|f| f.name == "date").unwrap();
        assert_eq!(date.field_type, FieldType::Date);
        assert_eq!(date.date_format, Some("%d/%m/%Y".to_string()));
    }

    #[test]
    fn discover_non_date_has_no_format() {
        let fm1 = json!({"title": "Hello", "count": 42});

        let inputs: Vec<(&str, Option<&serde_json::Value>)> = vec![("a.md", Some(&fm1))];
        let fields = discover_fields(&inputs, DEFAULT_DATE_FORMATS);

        for f in &fields {
            assert_eq!(f.date_format, None);
        }
    }
}
