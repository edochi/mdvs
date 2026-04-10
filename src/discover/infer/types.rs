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
    /// Distinct non-null values observed (element-level for arrays).
    pub distinct_values: Vec<Value>,
    /// Total non-null value count (element-level for arrays).
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

/// Collect distinct values and occurrence counts for a single field value.
/// For arrays, collects at element level. For scalars, collects the value itself.
fn collect_distinct_values(
    key: &str,
    val: &Value,
    distinct: &mut HashMap<String, Vec<Value>>,
    counts: &mut HashMap<String, usize>,
) {
    match val {
        Value::Array(arr) => {
            let entry = distinct.entry(key.to_string()).or_default();
            for elem in arr {
                if !elem.is_null() {
                    if !entry.contains(elem) {
                        entry.push(elem.clone());
                    }
                    *counts.entry(key.to_string()).or_default() += 1;
                }
            }
        }
        _ => {
            let entry = distinct.entry(key.to_string()).or_default();
            if !entry.contains(val) {
                entry.push(val.clone());
            }
            *counts.entry(key.to_string()).or_default() += 1;
        }
    }
}
