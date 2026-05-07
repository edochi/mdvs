//! Type inference — flat pass over all files, widening field types across occurrences.

use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use tracing::instrument;

/// Widened type and file list for a single frontmatter field.
#[derive(Debug)]
pub struct FieldTypeInfo {
    /// Widened type across all files where this field appears.
    pub field_type: FieldType,
    /// Paths of files containing this field.
    pub files: Vec<PathBuf>,
    /// Whether any file had a null value for this field.
    pub nullable: bool,
    /// Distinct non-null values, post-processed for the widened type:
    /// element-level for Array fields, serialized for String fields.
    pub distinct_values: Vec<Value>,
    /// Total non-null value count (element-level for Array fields,
    /// value-level for all others).
    pub occurrence_count: usize,
}

/// Infer field types by scanning all files and widening across occurrences.
/// Also collects distinct values and occurrence counts for categorical inference.
#[instrument(name = "infer_types", skip_all, level = "debug")]
pub fn infer_field_types(scanned: &ScannedFiles) -> BTreeMap<String, FieldTypeInfo> {
    let mut types: BTreeMap<String, FieldType> = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    let mut nulls: HashSet<String> = HashSet::new();
    let mut distinct: HashMap<String, Vec<Value>> = HashMap::new();
    let mut counts: HashMap<String, usize> = HashMap::new();

    for file in &scanned.files {
        if let Some(Value::Object(map)) = &file.data {
            for (key, val) in map {
                // Always track file presence
                files
                    .entry(key.clone())
                    .or_default()
                    .push(file.path.clone());

                // Skip null — transparent in type inference
                if val.is_null() {
                    nulls.insert(key.clone());
                    continue;
                }

                let ft = FieldType::from(val);
                types
                    .entry(key.clone())
                    .and_modify(|existing| {
                        *existing = FieldType::from_widen(existing.clone(), ft.clone())
                    })
                    .or_insert(ft);

                // Collect distinct values for categorical inference
                collect_distinct_values(key, val, &mut distinct, &mut counts);
            }
        }
    }

    // Fields present only as null default to String
    for key in files.keys() {
        types.entry(key.clone()).or_insert(FieldType::String);
    }

    // Post-process distinct values based on the final widened types.
    // During collection, raw values were stored (arrays not expanded).
    // Now reconcile with the final type:
    //   Array(T) → expand arrays to elements, deduplicate, recount at element level
    //   String   → serialize non-string values with Value::to_string()
    //              (matching build's serialization at storage.rs)
    //   Other    → no change (can't arise from array+scalar widening)
    for (name, ft) in &types {
        let vals = match distinct.get_mut(name) {
            Some(v) => v,
            None => continue,
        };

        match ft {
            FieldType::Array(_) => {
                let mut elements: Vec<Value> = Vec::new();
                let mut element_count: usize = 0;
                for val in vals.iter() {
                    if let Value::Array(arr) = val {
                        for elem in arr {
                            if !elem.is_null() {
                                if !elements.contains(elem) {
                                    elements.push(elem.clone());
                                }
                                element_count += 1;
                            }
                        }
                    } else {
                        if !elements.contains(val) {
                            elements.push(val.clone());
                        }
                        element_count += 1;
                    }
                }
                *vals = elements;
                counts.insert(name.clone(), element_count);
            }
            FieldType::String => {
                let mut serialized: Vec<Value> = Vec::new();
                for val in vals.iter() {
                    let s = match val {
                        Value::String(_) => val.clone(),
                        other => Value::String(other.to_string()),
                    };
                    if !serialized.contains(&s) {
                        serialized.push(s);
                    }
                }
                *vals = serialized;
            }
            _ => {}
        }
    }

    types
        .into_iter()
        .map(|(name, field_type)| {
            let info = FieldTypeInfo {
                field_type,
                files: files.remove(&name).unwrap_or_default(),
                nullable: nulls.contains(&name),
                distinct_values: distinct.remove(&name).unwrap_or_default(),
                occurrence_count: counts.remove(&name).unwrap_or(0),
            };
            (name, info)
        })
        .collect()
}

/// Collect raw distinct values and occurrence counts for a single field value.
/// Always stores the value as-is (arrays are NOT expanded here).
/// Post-processing in `infer_field_types` reconciles with the final widened type.
fn collect_distinct_values(
    key: &str,
    val: &Value,
    distinct: &mut HashMap<String, Vec<Value>>,
    counts: &mut HashMap<String, usize>,
) {
    let entry = distinct.entry(key.to_string()).or_default();
    if !entry.contains(val) {
        entry.push(val.clone());
    }
    *counts.entry(key.to_string()).or_default() += 1;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::scan::{ScannedFile, ScannedFiles};
    use serde_json::json;

    fn sf(path: &str, data: Option<Value>) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(path),
            data,
            content: String::new(),
            body_line_offset: 0,
        }
    }

    #[test]
    fn mixed_string_array_widens_to_string_with_serialized_distinct() {
        // funding: "internal" in some files, funding: ["internal"] in others.
        // Type widens to String. Distinct values must include both the plain
        // string and the serialized array form.
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"funding": "internal"}))),
                sf("b.md", Some(json!({"funding": ["internal"]}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let funding = &info["funding"];
        assert_eq!(funding.field_type, FieldType::String);
        assert_eq!(funding.distinct_values.len(), 2);
        assert!(funding.distinct_values.contains(&json!("internal")));
        assert!(funding.distinct_values.contains(&json!(r#"["internal"]"#)));
        assert_eq!(funding.occurrence_count, 2);
    }

    #[test]
    fn pure_array_field_expands_elements() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"tags": ["rust", "go"]}))),
                sf("b.md", Some(json!({"tags": ["rust", "python"]}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let tags = &info["tags"];
        assert_eq!(
            tags.field_type,
            FieldType::Array(Box::new(FieldType::String))
        );
        assert_eq!(tags.distinct_values.len(), 3); // rust, go, python
        assert_eq!(tags.occurrence_count, 4); // 2 + 2 elements
    }

    #[test]
    fn pure_string_field_unchanged() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"status": "draft"}))),
                sf("b.md", Some(json!({"status": "published"}))),
                sf("c.md", Some(json!({"status": "draft"}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let status = &info["status"];
        assert_eq!(status.field_type, FieldType::String);
        assert_eq!(status.distinct_values.len(), 2);
        assert_eq!(status.occurrence_count, 3);
    }

    #[test]
    fn integer_field_unchanged() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"count": 1}))),
                sf("b.md", Some(json!({"count": 2}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let count = &info["count"];
        assert_eq!(count.field_type, FieldType::Integer);
        assert_eq!(count.distinct_values.len(), 2);
        assert_eq!(count.occurrence_count, 2);
    }

    // -----------------------------------------------------------------------
    // collect_distinct_values — raw collection (no expansion)
    // -----------------------------------------------------------------------

    #[test]
    fn collect_stores_raw_array_without_expanding() {
        let mut distinct: HashMap<String, Vec<Value>> = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        let arr = json!(["a", "b"]);
        collect_distinct_values("f", &arr, &mut distinct, &mut counts);
        // Raw array stored, not individual elements
        assert_eq!(distinct["f"].len(), 1);
        assert_eq!(distinct["f"][0], arr);
        assert_eq!(counts["f"], 1);
    }

    #[test]
    fn collect_deduplicates_identical_arrays() {
        let mut distinct: HashMap<String, Vec<Value>> = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        collect_distinct_values("f", &json!(["a"]), &mut distinct, &mut counts);
        collect_distinct_values("f", &json!(["a"]), &mut distinct, &mut counts);
        assert_eq!(distinct["f"].len(), 1);
        assert_eq!(counts["f"], 2);
    }

    #[test]
    fn collect_distinguishes_string_from_array() {
        let mut distinct: HashMap<String, Vec<Value>> = HashMap::new();
        let mut counts: HashMap<String, usize> = HashMap::new();
        collect_distinct_values("f", &json!("a"), &mut distinct, &mut counts);
        collect_distinct_values("f", &json!(["a"]), &mut distinct, &mut counts);
        assert_eq!(distinct["f"].len(), 2);
        assert_eq!(counts["f"], 2);
    }

    // -----------------------------------------------------------------------
    // Post-processing edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn empty_array_serialized_for_string_field() {
        // projects: [] in some files, projects: "X" in others → String.
        // Empty array serializes to "[]".
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"projects": "X"}))),
                sf("b.md", Some(json!({"projects": []}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let p = &info["projects"];
        assert_eq!(p.field_type, FieldType::String);
        assert_eq!(p.distinct_values.len(), 2);
        assert!(p.distinct_values.contains(&json!("X")));
        assert!(p.distinct_values.contains(&json!("[]")));
    }

    #[test]
    fn refractions_pattern_string_array_empty_array_null() {
        // Real-world pattern: mix of string, array, empty array, and null.
        // Null is transparent → doesn't appear in distinct.
        // String + Array → widens to String.
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"p": "X"}))),
                sf("b.md", Some(json!({"p": ["X"]}))),
                sf("c.md", Some(json!({"p": ["X", "Y"]}))),
                sf("d.md", Some(json!({"p": []}))),
                sf("e.md", Some(json!({"p": null}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let p = &info["p"];
        assert_eq!(p.field_type, FieldType::String);
        assert!(p.nullable);
        // 4 non-null distinct values: "X", '["X"]', '["X","Y"]', '[]'
        assert_eq!(p.distinct_values.len(), 4);
        assert!(p.distinct_values.contains(&json!("X")));
        assert!(p.distinct_values.contains(&json!("[]")));
        assert!(p.distinct_values.contains(&json!(r#"["X"]"#)));
        assert!(p.distinct_values.contains(&json!(r#"["X","Y"]"#)));
        // occurrence_count excludes null
        assert_eq!(p.occurrence_count, 4);
    }

    #[test]
    fn boolean_widened_to_string_serialized() {
        // draft: true in some files, draft: "yes" in others → String.
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"draft": true}))),
                sf("b.md", Some(json!({"draft": "yes"}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let d = &info["draft"];
        assert_eq!(d.field_type, FieldType::String);
        assert_eq!(d.distinct_values.len(), 2);
        assert!(d.distinct_values.contains(&json!("yes")));
        assert!(d.distinct_values.contains(&json!("true")));
    }

    #[test]
    fn integer_widened_to_string_serialized() {
        // count: 42 in some files, count: "many" in others → String.
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"count": 42}))),
                sf("b.md", Some(json!({"count": "many"}))),
            ],
        };
        let info = infer_field_types(&scanned);
        let c = &info["count"];
        assert_eq!(c.field_type, FieldType::String);
        assert_eq!(c.distinct_values.len(), 2);
        assert!(c.distinct_values.contains(&json!("many")));
        assert!(c.distinct_values.contains(&json!("42")));
    }

    #[test]
    fn array_with_null_elements_excludes_nulls_in_expansion() {
        // tags: ["a", null, "b"] — null elements excluded during expansion.
        let scanned = ScannedFiles {
            files: vec![sf("a.md", Some(json!({"tags": ["a", null, "b"]})))],
        };
        let info = infer_field_types(&scanned);
        let t = &info["tags"];
        assert_eq!(t.field_type, FieldType::Array(Box::new(FieldType::String)));
        assert_eq!(t.distinct_values.len(), 2); // "a", "b"
        assert_eq!(t.occurrence_count, 2); // null excluded
    }
}
