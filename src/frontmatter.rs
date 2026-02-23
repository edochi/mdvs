use std::collections::HashMap;

use gray_matter::engine::YAML;
use gray_matter::Matter;
use serde_json::Value;

use crate::types::{FieldInfo, FieldType};

/// Extract frontmatter and body from markdown content.
/// Detects TOML (`+++`) vs YAML (`---`) delimiters.
pub fn extract_frontmatter(content: &str) -> (Option<Value>, String) {
    let trimmed = content.trim_start();

    if trimmed.starts_with("+++") {
        extract_toml_frontmatter(content)
    } else {
        extract_yaml_frontmatter(content)
    }
}

fn extract_toml_frontmatter(content: &str) -> (Option<Value>, String) {
    let trimmed = content.trim_start();
    let after_open = &trimmed[3..];
    let Some(close_pos) = after_open.find("+++") else {
        return (None, content.to_string());
    };

    let toml_str = &after_open[..close_pos];
    let body = after_open[close_pos + 3..]
        .trim_start_matches('\n')
        .to_string();

    match toml_str.parse::<toml::Value>() {
        Ok(toml_val) => {
            let json = toml_to_json(&toml_val);
            (Some(json), body)
        }
        Err(_) => (None, content.to_string()),
    }
}

fn toml_to_json(val: &toml::Value) -> Value {
    match val {
        toml::Value::String(s) => Value::String(s.clone()),
        toml::Value::Integer(i) => Value::Number((*i).into()),
        toml::Value::Float(f) => {
            serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
        }
        toml::Value::Boolean(b) => Value::Bool(*b),
        toml::Value::Datetime(dt) => Value::String(dt.to_string()),
        toml::Value::Array(arr) => Value::Array(arr.iter().map(toml_to_json).collect()),
        toml::Value::Table(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), toml_to_json(v)))
                .collect();
            Value::Object(obj)
        }
    }
}

fn extract_yaml_frontmatter(content: &str) -> (Option<Value>, String) {
    let matter = Matter::<YAML>::new();
    let parsed = matter.parse(content);

    let data = parsed.data.and_then(|d| {
        let yaml: serde_yaml::Value = d.deserialize().ok()?;
        let json = yaml_to_json(&yaml);
        if json.is_object() {
            Some(json)
        } else {
            None
        }
    });

    (data, parsed.content)
}

fn yaml_to_json(val: &serde_yaml::Value) -> Value {
    match val {
        serde_yaml::Value::Null => Value::Null,
        serde_yaml::Value::Bool(b) => Value::Bool(*b),
        serde_yaml::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Number(i.into())
            } else if let Some(f) = n.as_f64() {
                serde_json::Number::from_f64(f).map_or(Value::Null, Value::Number)
            } else {
                Value::Null
            }
        }
        serde_yaml::Value::String(s) => Value::String(s.clone()),
        serde_yaml::Value::Sequence(seq) => Value::Array(seq.iter().map(yaml_to_json).collect()),
        serde_yaml::Value::Mapping(map) => {
            let obj: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| {
                    let key = match k {
                        serde_yaml::Value::String(s) => s.clone(),
                        other => format!("{other:?}"),
                    };
                    (key, yaml_to_json(v))
                })
                .collect();
            Value::Object(obj)
        }
        serde_yaml::Value::Tagged(tagged) => yaml_to_json(&tagged.value),
    }
}

/// Infer the FieldType for a JSON value.
fn infer_type(value: &Value) -> FieldType {
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
fn is_date_string(s: &str) -> bool {
    if s.len() < 10 {
        return false;
    }
    let bytes = s.as_bytes();
    bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[0..4].iter().all(u8::is_ascii_digit)
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[8..10].iter().all(u8::is_ascii_digit)
}

/// Scan all notes and discover fields with type inference.
pub fn discover_fields(notes: &[crate::types::NoteData]) -> Vec<FieldInfo> {
    let mut field_counts: HashMap<String, HashMap<FieldType, usize>> = HashMap::new();

    for note in notes {
        let Some(Value::Object(map)) = &note.frontmatter else {
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

/// Split a note's frontmatter into promoted values and remaining metadata.
pub fn split_frontmatter(fm: &Value, fields: &[FieldInfo]) -> (HashMap<String, Value>, Value) {
    let mut promoted = HashMap::new();
    let mut metadata = serde_json::Map::new();

    let Some(map) = fm.as_object() else {
        return (promoted, Value::Object(metadata));
    };

    let promoted_fields: HashMap<&str, &FieldInfo> = fields
        .iter()
        .filter(|f| f.promoted)
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
