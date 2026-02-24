use std::collections::HashMap;

use serde_json::Value;

use crate::FieldType;

/// A field discovered by scanning frontmatter across files.
#[derive(Debug, Clone)]
pub struct FieldInfo {
    /// Field name as it appears in frontmatter.
    pub name: String,
    /// Inferred type based on the most common value type seen.
    pub field_type: FieldType,
    /// Relative paths of files containing this field.
    pub files: Vec<String>,
}

/// Infer the FieldType for a JSON value.
pub fn infer_type(value: &Value) -> FieldType {
    match value {
        Value::Bool(_) => FieldType::Boolean,
        Value::Number(n) => {
            if n.is_i64() || n.is_u64() {
                FieldType::Integer
            } else {
                FieldType::Float
            }
        }
        Value::String(s) => {
            if is_date_string(s) {
                FieldType::Date
            } else {
                FieldType::String
            }
        }
        Value::Array(_) => FieldType::StringArray,
        _ => FieldType::String,
    }
}

/// Check if a string looks like a date (YYYY-MM-DD with optional time).
pub fn is_date_string(s: &str) -> bool {
    s.len() >= 10 && chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").is_ok()
}

/// Scan frontmatter values and discover fields with type inference.
///
/// Takes `(relative_path, frontmatter)` pairs. Tracks which files contain each field.
pub fn discover_fields(file_frontmatters: &[(&str, Option<&Value>)]) -> Vec<FieldInfo> {
    let mut field_types: HashMap<String, HashMap<FieldType, usize>> = HashMap::new();
    let mut field_files: HashMap<String, Vec<String>> = HashMap::new();

    for (path, fm) in file_frontmatters {
        let Some(Value::Object(map)) = fm else {
            continue;
        };
        for (key, val) in map {
            let ft = infer_type(val);
            *field_types
                .entry(key.clone())
                .or_default()
                .entry(ft)
                .or_insert(0) += 1;
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

            FieldInfo {
                name,
                field_type: field_type.clone(),
                files,
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
        assert!(!is_date_string("not-a-date"));
        assert!(!is_date_string("2025-13-01")); // invalid month
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
        let fields = discover_fields(&inputs);

        // title appears in all 3
        let title = fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title.files.len(), 3);
        assert!(title.files.contains(&"blog/a.md".to_string()));

        // date appears in 2
        let date = fields.iter().find(|f| f.name == "date").unwrap();
        assert_eq!(date.files.len(), 2);

        // author appears in 1
        let author = fields.iter().find(|f| f.name == "author").unwrap();
        assert_eq!(author.files.len(), 1);
        assert_eq!(author.files[0], "notes/c.md");
    }
}
