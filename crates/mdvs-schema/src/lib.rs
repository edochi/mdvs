use std::collections::HashMap;
use std::fmt;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldType {
    String,
    StringArray,
    Date,
    Boolean,
    Integer,
    Float,
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::String => write!(f, "String"),
            FieldType::StringArray => write!(f, "String[]"),
            FieldType::Date => write!(f, "Date"),
            FieldType::Boolean => write!(f, "Boolean"),
            FieldType::Integer => write!(f, "Integer"),
            FieldType::Float => write!(f, "Float"),
        }
    }
}

impl FieldType {
    pub fn sql_type(&self) -> &'static str {
        match self {
            FieldType::String => "VARCHAR",
            FieldType::StringArray => "VARCHAR[]",
            FieldType::Date => "DATE",
            FieldType::Boolean => "BOOLEAN",
            FieldType::Integer => "BIGINT",
            FieldType::Float => "DOUBLE",
        }
    }
}

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
    s.len() >= 10
        && chrono::NaiveDate::parse_from_str(&s[..10], "%Y-%m-%d").is_ok()
}

/// Scan frontmatter values and discover fields with type inference.
/// Accepts a slice of `Option<&Value>` so callers can pass frontmatter
/// without depending on any specific note type.
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
            // Majority type wins
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
