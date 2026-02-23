use std::collections::HashMap;

use serde_json::Value;

use crate::FieldType;

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub field_type: FieldType,
    pub count: usize,
    pub promoted: bool,
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
pub fn discover_fields(frontmatters: &[Option<&Value>]) -> Vec<FieldInfo> {
    let mut field_counts: HashMap<String, HashMap<FieldType, usize>> = HashMap::new();

    for fm in frontmatters {
        let Some(Value::Object(map)) = fm else {
            continue;
        };
        for (key, val) in map {
            let ft = infer_type(val);
            *field_counts
                .entry(key.clone())
                .or_default()
                .entry(ft)
                .or_insert(0) += 1;
        }
    }

    let mut fields: Vec<FieldInfo> = field_counts
        .into_iter()
        .map(|(name, type_counts)| {
            let (field_type, _) = type_counts.iter().max_by_key(|(_, count)| *count).unwrap();
            let total_count: usize = type_counts.values().sum();

            FieldInfo {
                name,
                field_type: field_type.clone(),
                count: total_count,
                promoted: false,
            }
        })
        .collect();

    fields.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    fields
}

/// Mark fields appearing in more than `threshold` fraction of files as promoted.
pub fn auto_promote(fields: &mut [FieldInfo], total_files: usize, threshold: f64) {
    let min_count = (total_files as f64 * threshold).ceil() as usize;
    for field in fields.iter_mut() {
        field.promoted = field.count >= min_count;
    }
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
    fn discover_and_promote() {
        let fm1 = json!({"title": "A", "tags": ["x"], "date": "2025-01-01"});
        let fm2 = json!({"title": "B", "date": "2025-01-02"});
        let fm3 = json!({"title": "C", "author": "me"});

        let fms: Vec<Option<&serde_json::Value>> = vec![Some(&fm1), Some(&fm2), Some(&fm3)];
        let mut fields = discover_fields(&fms);

        // title appears in all 3
        let title = fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title.count, 3);

        auto_promote(&mut fields, 3, 0.5);
        let title = fields.iter().find(|f| f.name == "title").unwrap();
        assert!(title.promoted);
        let date = fields.iter().find(|f| f.name == "date").unwrap();
        assert!(date.promoted); // 2/3 >= 0.5
        let author = fields.iter().find(|f| f.name == "author").unwrap();
        assert!(!author.promoted); // 1/3 < 0.5
    }
}
