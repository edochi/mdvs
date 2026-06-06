//! `canonical_to_dsl` — reverse translator that turns a canonical JSON
//! Schema document (presumed to have passed
//! [`super::validate_mdvs_schema`]) back into mdvs DSL fields.
//!
//! Used by `mdvs init --from-jsonschema`. Strict pattern matching: only
//! the exact shapes [`super::to_canonical::dsl_to_canonical`] produces are
//! accepted. Anything else errors with a clear message pointing at the
//! property name.

use crate::discover::field_type::FieldType;
use crate::schema::config::TomlField;
use crate::schema::constraints::Constraints;
use serde_json::{Map, Value};

use super::is_intermediate_object;

/// Output of [`canonical_to_dsl`]: the per-property fields plus any
/// empty-schema entries that map to the `[fields].ignore` list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanonicalImport {
    pub fields: Vec<TomlField>,
    pub ignore: Vec<String>,
}

/// Translate a canonical JSON Schema back into mdvs DSL fields.
///
/// **Dotted-name reconstruction** (TODO-0097 step 3): the nested
/// `properties` tree is walked with a path prefix. Intermediate Object
/// nodes (those produced by `intermediate_object_schema` — `{type:
/// "object", properties: {...}, additionalProperties: true}` with no
/// `x-mdvs`) are recursed into, extending the dotted prefix. Leaf
/// schemas become one `TomlField` each with the full dotted path as
/// its name.
///
/// Ignored names are emitted only at the top level (they were flat in
/// `dsl_to_canonical`'s output).
pub(crate) fn canonical_to_dsl(schema: &Value) -> Result<CanonicalImport, String> {
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| "schema is missing 'properties' block".to_string())?;

    let mut fields: Vec<TomlField> = Vec::new();
    let mut ignore: Vec<String> = Vec::new();

    walk_properties_into_fields(properties, "", &mut fields, &mut ignore, true)?;

    Ok(CanonicalImport { fields, ignore })
}

/// Recursive walk used by [`canonical_to_dsl`]. `prefix` is the dotted-path
/// accumulator. `top_level` is true only for the outermost call — ignored
/// (empty) schemas are recognised only there, since `dsl_to_canonical`
/// emits them flat at the root.
fn walk_properties_into_fields(
    properties: &Map<String, Value>,
    prefix: &str,
    fields: &mut Vec<TomlField>,
    ignore: &mut Vec<String>,
    top_level: bool,
) -> Result<(), String> {
    for (name, sub) in properties {
        let full_name = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };

        let subobj = sub
            .as_object()
            .ok_or_else(|| format!("property '{full_name}': schema must be an object"))?;

        // Top-level empty schemas are ignored names; nested ones would be a
        // malformed schema (intermediates always have `type: "object"` and
        // children, leaves always have type info or x-mdvs).
        if subobj.is_empty() {
            if top_level {
                ignore.push(name.clone());
                continue;
            }
            return Err(format!(
                "property '{full_name}': empty schema is not allowed at nested depth"
            ));
        }

        if is_intermediate_object(sub) {
            // Intermediate Object — recurse into children, extending prefix.
            // is_intermediate_object guarantees `properties` exists as an
            // object; the explicit `?`-style guard below is defensive.
            let Some(inner) = sub.get("properties").and_then(Value::as_object) else {
                return Err(format!(
                    "property '{full_name}': internal error — intermediate object missing properties"
                ));
            };
            walk_properties_into_fields(inner, &full_name, fields, ignore, false)?;
        } else {
            // Leaf — emit a TomlField with the dotted full path.
            fields.push(field_from_subschema(&full_name, subobj)?);
        }
    }
    Ok(())
}

fn field_from_subschema(name: &str, sub: &Map<String, Value>) -> Result<TomlField, String> {
    let (field_type, nullable) = extract_type(name, sub)?;

    // Locate the constraint-bearing subschema: for arrays, constraints live
    // inside `items` (per dsl_to_canonical); otherwise at the property level.
    let constraint_source = if matches!(field_type, FieldType::Array(_)) {
        sub.get("items")
            .and_then(Value::as_object)
            .ok_or_else(|| format!("property '{name}': array type missing 'items' object"))?
    } else {
        sub
    };
    let constraints = extract_constraints(name, constraint_source, nullable)?;

    let (allowed, required, preprocess) = extract_x_mdvs(name, sub)?;

    Ok(TomlField {
        name: name.into(),
        field_type: crate::schema::shared::FieldTypeSerde::from(&field_type),
        allowed,
        required,
        nullable,
        constraints,
        preprocess,
    })
}

/// Extract the FieldType and nullability from a subschema's `type` keyword
/// (and `items` recursion for arrays).
fn extract_type(name: &str, sub: &Map<String, Value>) -> Result<(FieldType, bool), String> {
    let type_val = sub
        .get("type")
        .ok_or_else(|| format!("property '{name}': missing 'type' keyword"))?;

    let (type_str, nullable) = match type_val {
        Value::String(s) => (s.as_str().to_string(), false),
        Value::Array(arr) => {
            let mut non_null: Vec<String> = Vec::new();
            let mut has_null = false;
            for v in arr {
                match v.as_str() {
                    Some("null") => has_null = true,
                    Some(s) => non_null.push(s.into()),
                    None => {
                        return Err(format!(
                            "property '{name}': type array contains a non-string entry"
                        ));
                    }
                }
            }
            if non_null.len() != 1 {
                return Err(format!(
                    "property '{name}': type union must be exactly one non-null type plus optional null"
                ));
            }
            // The length check above guarantees exactly one element; the
            // `unwrap_or_default` fallback is defensive.
            (non_null.into_iter().next().unwrap_or_default(), has_null)
        }
        _ => {
            return Err(format!(
                "property '{name}': 'type' must be a string or array of strings"
            ));
        }
    };

    let field_type = match type_str.as_str() {
        "string" => match sub.get("format").and_then(Value::as_str) {
            Some("date") => FieldType::Date,
            Some("date-time") => FieldType::DateTime,
            Some(other) => {
                return Err(format!(
                    "property '{name}': unsupported format '{other}' on string type"
                ));
            }
            None => FieldType::String,
        },
        "integer" => FieldType::Integer,
        "number" => FieldType::Float,
        "boolean" => FieldType::Boolean,
        "array" => {
            let items = sub.get("items").and_then(Value::as_object).ok_or_else(|| {
                format!("property '{name}': type 'array' requires 'items' object")
            })?;
            let (inner, _) = extract_type(name, items)?;
            FieldType::Array(Box::new(inner))
        }
        "object" => {
            // After TODO-0097 step 3, an "object" type at this layer is
            // either an intermediate (which canonical_to_dsl recurses into
            // before reaching extract_type) or an Array's inner Object
            // (carrying real `properties` children we must reconstruct).
            //
            // Read children if present; otherwise produce an empty Object.
            let mut children = std::collections::BTreeMap::new();
            if let Some(inner_props) = sub.get("properties").and_then(Value::as_object) {
                for (child_name, child_sub) in inner_props {
                    let child_obj = child_sub.as_object().ok_or_else(|| {
                        format!(
                            "property '{name}': inner Object child '{child_name}' schema must be an object"
                        )
                    })?;
                    let (child_ft, _) = extract_type(child_name, child_obj)?;
                    children.insert(child_name.clone(), child_ft);
                }
            }
            FieldType::Object(children)
        }
        other => {
            return Err(format!("property '{name}': unsupported type '{other}'"));
        }
    };

    Ok((field_type, nullable))
}

/// Extract `Constraints` from the property-level (or items-level) subschema.
/// Strips a trailing null from enum lists when the field is nullable
/// (inverting the addition done by `apply_constraints`).
fn extract_constraints(
    name: &str,
    src: &Map<String, Value>,
    nullable: bool,
) -> Result<Option<Constraints>, String> {
    let has_any = [
        "enum",
        "minimum",
        "maximum",
        "minLength",
        "maxLength",
        "pattern",
    ]
    .iter()
    .any(|k| src.contains_key(*k));
    if !has_any {
        return Ok(None);
    }

    let mut c = Constraints::default();

    if let Some(en) = src.get("enum") {
        let arr = en
            .as_array()
            .ok_or_else(|| format!("property '{name}': 'enum' must be an array"))?;
        let mut values: Vec<toml::Value> = Vec::new();
        for v in arr {
            if v.is_null() {
                if !nullable {
                    return Err(format!(
                        "property '{name}': 'enum' contains null but type is not nullable"
                    ));
                }
                // Strip — added by dsl_to_canonical for nullable+enum fields.
                continue;
            }
            values.push(
                json_to_toml_value(v)
                    .ok_or_else(|| format!("property '{name}': unsupported enum value {v}"))?,
            );
        }
        c.categories = Some(values);
    }

    if let Some(min) = src.get("minimum") {
        c.min = Some(
            json_to_toml_value(min)
                .ok_or_else(|| format!("property '{name}': unsupported 'minimum' value"))?,
        );
    }
    if let Some(max) = src.get("maximum") {
        c.max = Some(
            json_to_toml_value(max)
                .ok_or_else(|| format!("property '{name}': unsupported 'maximum' value"))?,
        );
    }
    if let Some(v) = src.get("minLength").and_then(Value::as_u64) {
        c.min_length = Some(v);
    }
    if let Some(v) = src.get("maxLength").and_then(Value::as_u64) {
        c.max_length = Some(v);
    }
    if let Some(v) = src.get("pattern").and_then(Value::as_str) {
        c.pattern = Some(v.into());
    }

    Ok(Some(c))
}

/// Inverse of `build_x_mdvs`: extract `allowed` / `required` / `preprocess`
/// from the property's `x-mdvs` block.
/// Defaults: `allowed = ["**"]`, `required = []`, `preprocess = []`.
#[allow(clippy::type_complexity)] // 3-tuple is the natural shape; one struct just for this would obscure usage
fn extract_x_mdvs(
    name: &str,
    sub: &Map<String, Value>,
) -> Result<(Vec<String>, Vec<String>, Vec<crate::preprocess::ValueStage>), String> {
    let xm = match sub.get("x-mdvs") {
        None => return Ok((vec!["**".into()], vec![], vec![])),
        Some(v) => v
            .as_object()
            .ok_or_else(|| format!("property '{name}': 'x-mdvs' must be an object"))?,
    };

    let allowed = match xm.get("allowed") {
        None => vec!["**".into()],
        Some(v) => string_array(v).ok_or_else(|| {
            format!("property '{name}': 'x-mdvs.allowed' must be an array of strings")
        })?,
    };
    let required = match xm.get("required") {
        None => vec![],
        Some(v) => string_array(v).ok_or_else(|| {
            format!("property '{name}': 'x-mdvs.required' must be an array of strings")
        })?,
    };
    let preprocess = match xm.get("preprocess") {
        None => vec![],
        Some(v) => serde_json::from_value::<Vec<crate::preprocess::ValueStage>>(v.clone())
            .map_err(|e| format!("property '{name}': invalid 'x-mdvs.preprocess' entry: {e}"))?,
    };
    Ok((allowed, required, preprocess))
}

fn string_array(v: &Value) -> Option<Vec<String>> {
    let arr = v.as_array()?;
    arr.iter().map(|s| s.as_str().map(String::from)).collect()
}

/// Convert a `serde_json::Value` to a `toml::Value` for round-tripping
/// constraint literals (enum values, min/max bounds).
fn json_to_toml_value(v: &Value) -> Option<toml::Value> {
    match v {
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Bool(b) => Some(toml::Value::Boolean(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else {
                n.as_f64().map(toml::Value::Float)
            }
        }
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}
