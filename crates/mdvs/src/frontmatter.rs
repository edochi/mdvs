use std::collections::HashMap;

use mdvs_schema::{FieldInfo, FieldType};
use serde_json::Value;

/// Split a note's frontmatter into promoted values and remaining metadata.
pub fn split_frontmatter(fm: &Value, fields: &[FieldInfo]) -> (HashMap<String, Value>, Value) {
    let mut promoted = HashMap::new();
    let mut metadata = serde_json::Map::new();

    let Some(map) = fm.as_object() else {
        return (promoted, Value::Object(metadata));
    };

    let promoted_fields: HashMap<&str, &FieldInfo> = fields
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();

    for (key, val) in map {
        if let Some(field_info) = promoted_fields.get(key.as_str()) {
            // Coerce value to match the promoted type
            let coerced = coerce_value(val, &field_info.field_type);
            promoted.insert(key.clone(), coerced);
        } else {
            metadata.insert(key.clone(), val.clone());
        }
    }

    (promoted, Value::Object(metadata))
}

/// Coerce a value to match the expected field type.
/// Scalar where array expected → wrap in array.
/// Date string kept as string (DB will cast).
fn coerce_value(val: &Value, target: &FieldType) -> Value {
    match target {
        FieldType::StringArray => match val {
            Value::Array(_) => val.clone(),
            // Scalar → wrap in single-element array
            other => Value::Array(vec![other.clone()]),
        },
        FieldType::Date => match val {
            Value::String(_) => val.clone(),
            other => Value::String(other.to_string()),
        },
        _ => val.clone(),
    }
}
