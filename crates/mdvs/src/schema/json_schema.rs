//! Translation between the `mdvs.toml` DSL and canonical JSON Schema 2020-12,
//! plus a validation gate that rejects schemas using keywords mdvs doesn't support.
//!
//! Two entry points:
//! - [`dsl_to_canonical`] — `MdvsToml` → `serde_json::Value` (a JSON Schema document).
//! - [`validate_mdvs_schema`] — checks an arbitrary JSON Schema is within the mdvs subset.
//!
//! Path-scoped validation (per-file `allowed`/`required` globs) is carried as
//! `x-mdvs.allowed` / `x-mdvs.required` on each property; the per-file overlay
//! synthesizer (TODO-0149 step 13) turns those into actual JSON Schema `required`
//! arrays at validation time.
#![allow(dead_code)] // callers wire in at TODO-0149 steps 4, 9, 10

use crate::discover::field_type::FieldType;
use crate::schema::config::MdvsToml;
use crate::schema::constraints::Constraints;
use serde_json::{Map, Value, json};

/// JSON Schema 2020-12 `$schema` URI.
const JSON_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Recognized `x-mdvs` sub-keys at the schema (root) level.
pub(crate) const MDVS_KEYS_SCHEMA: &[&str] = &["preprocess", "definitions"];

/// Recognized `x-mdvs` sub-keys at the property level.
pub(crate) const MDVS_KEYS_PROPERTY: &[&str] = &["allowed", "required", "preprocess"];

/// JSON Schema keywords mdvs supports.
const ALLOW_LIST: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
    "pattern",
    "minItems",
    "maxItems",
    "uniqueItems",
    "$schema",
    "$id",
    "title",
    "description",
    "x-mdvs",
];

/// Common JSON Schema keywords mdvs explicitly does not support, paired with
/// a user-facing reason. Catching these by name produces a better error than
/// the generic "unknown keyword" path.
const HARD_REJECT: &[(&str, &str)] = &[
    (
        "oneOf",
        "composition keywords are out of scope; use path-scoped fields (x-mdvs.allowed/required) instead",
    ),
    ("anyOf", "composition keywords are out of scope"),
    ("allOf", "composition keywords are out of scope"),
    ("not", "composition keywords are out of scope"),
    ("if", "conditional keywords are out of scope"),
    ("then", "conditional keywords are out of scope"),
    ("else", "conditional keywords are out of scope"),
    (
        "$ref",
        "reference keywords are out of scope; mdvs schemas are self-contained",
    ),
    ("$defs", "reference keywords are out of scope"),
    ("dependentRequired", "dependent keywords are out of scope"),
    ("dependentSchemas", "dependent keywords are out of scope"),
    (
        "patternProperties",
        "patternProperties is out of scope; declare each field explicitly",
    ),
    (
        "prefixItems",
        "prefixItems (tuple validation) is out of scope; use uniform items",
    ),
    (
        "format",
        "format is out of scope; use pattern for regex-based validation",
    ),
    ("contains", "contains is out of scope"),
    ("propertyNames", "propertyNames is out of scope"),
];

// ============================================================================
// dsl_to_canonical
// ============================================================================

/// Translate an `MdvsToml` DSL into a canonical JSON Schema 2020-12 document.
///
/// The output has:
/// - `type: "object"`, `additionalProperties: true` at the root (per-file
///   overlay synthesis tightens this — see TODO-0149 step 13).
/// - One entry under `properties` per `[[fields.field]]` and per `ignore` name.
/// - Path-scoping carried as `x-mdvs.allowed` / `x-mdvs.required` on each property.
/// - No root-level `required` array — requirement is path-scoped.
pub(crate) fn dsl_to_canonical(toml: &MdvsToml) -> Value {
    let mut properties: Map<String, Value> = Map::new();

    for field in &toml.fields.field {
        properties.insert(field.name.clone(), field_to_subschema(field));
    }
    for ignored in &toml.fields.ignore {
        // Empty schema = always-passes. Step 13's overlay may further constrain.
        properties.insert(ignored.clone(), json!({}));
    }

    json!({
        "$schema": JSON_SCHEMA_DRAFT,
        "type": "object",
        "properties": Value::Object(properties),
        "additionalProperties": true,
    })
}

fn field_to_subschema(field: &crate::schema::config::TomlField) -> Value {
    let ft = match FieldType::try_from(&field.field_type) {
        Ok(ft) => ft,
        // Unparseable types are caught by `MdvsToml::validate()` before this
        // function is reachable. If we get here, fall through to an empty schema.
        Err(_) => return json!({}),
    };

    let mut subschema = type_subschema(&ft, field.nullable, field.constraints.as_ref());

    let x_mdvs = build_x_mdvs(field);
    if !x_mdvs.is_empty() {
        let map = subschema
            .as_object_mut()
            .expect("type_subschema always returns an object");
        map.insert("x-mdvs".to_string(), Value::Object(x_mdvs));
    }

    subschema
}

/// Produce the JSON Schema for a field type, applying `nullable` and any
/// `Constraints` at the appropriate level (scalar fields apply constraints to
/// the scalar; array fields apply constraints to `items`).
fn type_subschema(ft: &FieldType, nullable: bool, constraints: Option<&Constraints>) -> Value {
    match ft {
        FieldType::Boolean => scalar_subschema("boolean", nullable, None),
        FieldType::Integer => scalar_subschema("integer", nullable, constraints),
        FieldType::Float => scalar_subschema("number", nullable, constraints),
        FieldType::String => scalar_subschema("string", nullable, constraints),
        FieldType::Array(inner) => {
            // Array constraints apply to items (per Constraints docstring:
            // categories applies to Array(String)/Array(Integer); range applies
            // to Array(Integer)/Array(Float)).
            let items = type_subschema(inner, false, constraints);
            let mut obj = Map::new();
            insert_type(&mut obj, "array", nullable);
            obj.insert("items".into(), items);
            Value::Object(obj)
        }
        FieldType::Object(map) => {
            let mut props = Map::new();
            for (k, v) in map {
                // Object children carry no constraints in v0 (Constraints
                // doesn't model nested objects).
                props.insert(k.clone(), type_subschema(v, false, None));
            }
            let mut obj = Map::new();
            insert_type(&mut obj, "object", nullable);
            obj.insert("properties".into(), Value::Object(props));
            obj.insert("additionalProperties".into(), Value::Bool(true));
            Value::Object(obj)
        }
    }
}

fn scalar_subschema(ty: &str, nullable: bool, constraints: Option<&Constraints>) -> Value {
    let mut obj = Map::new();
    insert_type(&mut obj, ty, nullable);
    if let Some(c) = constraints {
        if let Some(cats) = &c.categories {
            obj.insert("enum".into(), categories_to_json(cats));
        }
        if let Some(min) = &c.min
            && let Some(v) = toml_to_json(min)
        {
            obj.insert("minimum".into(), v);
        }
        if let Some(max) = &c.max
            && let Some(v) = toml_to_json(max)
        {
            obj.insert("maximum".into(), v);
        }
    }
    Value::Object(obj)
}

fn insert_type(obj: &mut Map<String, Value>, ty: &str, nullable: bool) {
    if nullable {
        obj.insert(
            "type".into(),
            Value::Array(vec![Value::String(ty.into()), Value::String("null".into())]),
        );
    } else {
        obj.insert("type".into(), Value::String(ty.into()));
    }
}

fn categories_to_json(cats: &[toml::Value]) -> Value {
    Value::Array(cats.iter().filter_map(toml_to_json).collect())
}

/// Convert a `toml::Value` to a `serde_json::Value` for JSON Schema literals
/// (`enum`, `minimum`, `maximum`). Returns `None` for shapes that have no JSON
/// equivalent — those are caught upstream by `Constraints::validate_config`.
fn toml_to_json(v: &toml::Value) -> Option<Value> {
    match v {
        toml::Value::String(s) => Some(Value::String(s.clone())),
        toml::Value::Integer(i) => Some(json!(i)),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f).map(Value::Number),
        toml::Value::Boolean(b) => Some(Value::Bool(*b)),
        toml::Value::Datetime(_) | toml::Value::Array(_) | toml::Value::Table(_) => None,
    }
}

fn build_x_mdvs(field: &crate::schema::config::TomlField) -> Map<String, Value> {
    let mut out = Map::new();
    if !is_default_allowed(&field.allowed) {
        out.insert(
            "allowed".into(),
            Value::Array(field.allowed.iter().cloned().map(Value::String).collect()),
        );
    }
    if !field.required.is_empty() {
        out.insert(
            "required".into(),
            Value::Array(field.required.iter().cloned().map(Value::String).collect()),
        );
    }
    out
}

fn is_default_allowed(allowed: &[String]) -> bool {
    allowed.len() == 1 && allowed[0] == "**"
}

// ============================================================================
// validate_mdvs_schema
// ============================================================================

/// Walk a JSON Schema and reject anything outside the mdvs subset.
///
/// Allowed keywords come from [`ALLOW_LIST`]; common deny-list entries
/// produce a specific error via [`HARD_REJECT`]. `x-mdvs` sub-keys are
/// constrained by location: schema-level vs property-level.
pub(crate) fn validate_mdvs_schema(schema: &Value) -> Result<(), String> {
    walk(schema, Location::Root)
}

#[derive(Clone, Copy)]
enum Location {
    /// Top-level schema document.
    Root,
    /// A property directly under the root `properties` object.
    RootProperty,
    /// Anywhere else (nested objects, items, etc.).
    Nested,
}

fn walk(node: &Value, location: Location) -> Result<(), String> {
    let obj = match node.as_object() {
        Some(o) => o,
        None => return Ok(()), // Boolean schemas (true/false) and scalars allowed inside enum/const/etc.
    };

    for (key, value) in obj {
        // Hard-reject list (specific message).
        if let Some((_, reason)) = HARD_REJECT.iter().find(|(k, _)| k == key) {
            return Err(format!("'{key}' is not supported by mdvs — {reason}"));
        }
        // Allow-list catch-all.
        if !ALLOW_LIST.contains(&key.as_str()) {
            return Err(format!(
                "unknown keyword '{key}' is not part of the mdvs schema subset"
            ));
        }

        match key.as_str() {
            "properties" => {
                let props = value
                    .as_object()
                    .ok_or_else(|| "'properties' must be an object".to_string())?;
                for (_, prop_schema) in props {
                    let child_loc = match location {
                        Location::Root => Location::RootProperty,
                        _ => Location::Nested,
                    };
                    walk(prop_schema, child_loc)?;
                }
            }
            "items" => {
                walk(value, Location::Nested)?;
            }
            "additionalProperties" => {
                // Allowed values: bool or schema. If schema, walk it.
                if value.is_object() {
                    walk(value, Location::Nested)?;
                }
            }
            "x-mdvs" => {
                let xm = value
                    .as_object()
                    .ok_or_else(|| "'x-mdvs' must be an object".to_string())?;
                let allowed_subkeys = match location {
                    Location::Root => MDVS_KEYS_SCHEMA,
                    Location::RootProperty => MDVS_KEYS_PROPERTY,
                    Location::Nested => {
                        return Err(
                            "'x-mdvs' is only valid at the schema root or on a root-level property"
                                .to_string(),
                        );
                    }
                };
                for k in xm.keys() {
                    if !allowed_subkeys.contains(&k.as_str()) {
                        return Err(format!(
                            "unknown 'x-mdvs.{k}' sub-key (recognized: {allowed_subkeys:?})"
                        ));
                    }
                }
            }
            _ => {} // scalars and arrays under enum/const etc. need no recursion
        }
    }

    Ok(())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::{FieldTypeSerde, ScanConfig};

    fn empty_toml() -> MdvsToml {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        }
    }

    fn with_fields(fields: Vec<TomlField>) -> MdvsToml {
        let mut t = empty_toml();
        t.fields.field = fields;
        t
    }

    fn field(name: &str, ft: FieldTypeSerde) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: ft,
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
        }
    }

    // ------------------------------------------------------------------------
    // Group A — dsl_to_canonical happy path
    // ------------------------------------------------------------------------

    #[test]
    fn empty_config_produces_empty_object_schema() {
        let out = dsl_to_canonical(&empty_toml());
        assert_eq!(out["$schema"], JSON_SCHEMA_DRAFT);
        assert_eq!(out["type"], "object");
        assert_eq!(out["additionalProperties"], true);
        assert_eq!(out["properties"], json!({}));
    }

    #[test]
    fn string_field_simple() {
        let toml = with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["title"], json!({"type": "string"}));
    }

    #[test]
    fn integer_field_simple() {
        let toml = with_fields(vec![field(
            "count",
            FieldTypeSerde::Scalar("Integer".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["count"], json!({"type": "integer"}));
    }

    #[test]
    fn float_field_simple() {
        let toml = with_fields(vec![field("score", FieldTypeSerde::Scalar("Float".into()))]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["score"], json!({"type": "number"}));
    }

    #[test]
    fn boolean_field_simple() {
        let toml = with_fields(vec![field(
            "draft",
            FieldTypeSerde::Scalar("Boolean".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["draft"], json!({"type": "boolean"}));
    }

    #[test]
    fn nullable_field_emits_union_type() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({"type": ["string", "null"]})
        );
    }

    #[test]
    fn array_of_strings_field() {
        let toml = with_fields(vec![field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["tags"],
            json!({"type": "array", "items": {"type": "string"}})
        );
    }

    #[test]
    fn object_field_recurses() {
        use std::collections::BTreeMap;
        let toml = with_fields(vec![field(
            "meta",
            FieldTypeSerde::Object {
                object: BTreeMap::from([
                    ("author".into(), FieldTypeSerde::Scalar("String".into())),
                    ("version".into(), FieldTypeSerde::Scalar("Integer".into())),
                ]),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["meta"],
            json!({
                "type": "object",
                "properties": {
                    "author": {"type": "string"},
                    "version": {"type": "integer"}
                },
                "additionalProperties": true
            })
        );
    }

    #[test]
    fn categories_constraint_emits_enum() {
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["status"],
            json!({"type": "string", "enum": ["draft", "published"]})
        );
    }

    #[test]
    fn range_constraint_emits_minimum_maximum() {
        let mut f = field("rating", FieldTypeSerde::Scalar("Integer".into()));
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(5)),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["rating"],
            json!({"type": "integer", "minimum": 0, "maximum": 5})
        );
    }

    #[test]
    fn array_constraint_applies_to_items() {
        let mut f = field(
            "scores",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Integer".into())),
            },
        );
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(100)),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["scores"],
            json!({
                "type": "array",
                "items": {"type": "integer", "minimum": 0, "maximum": 100}
            })
        );
    }

    #[test]
    fn path_scoping_emitted_as_x_mdvs() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.allowed = vec!["blog/**".into(), "notes/**".into()];
        f.required = vec!["blog/**".into()];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({
                "type": "string",
                "x-mdvs": {
                    "allowed": ["blog/**", "notes/**"],
                    "required": ["blog/**"]
                }
            })
        );
    }

    #[test]
    fn default_allowed_omits_x_mdvs_block() {
        // allowed = ["**"] (default) and no required → no x-mdvs at all.
        let out = dsl_to_canonical(&with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]));
        assert!(out["properties"]["title"].get("x-mdvs").is_none());
    }

    #[test]
    fn ignore_list_produces_empty_subschemas() {
        let mut t = empty_toml();
        t.fields.ignore = vec!["internal_id".into(), "draft_meta".into()];
        let out = dsl_to_canonical(&t);
        assert_eq!(out["properties"]["internal_id"], json!({}));
        assert_eq!(out["properties"]["draft_meta"], json!({}));
    }

    #[test]
    fn no_root_required_array_emitted() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.required = vec!["**".into()];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert!(out.get("required").is_none());
    }

    // ------------------------------------------------------------------------
    // Group B — dsl_to_canonical output passes the gate
    // ------------------------------------------------------------------------

    #[test]
    fn gate_accepts_empty_schema() {
        let out = dsl_to_canonical(&empty_toml());
        assert!(validate_mdvs_schema(&out).is_ok());
    }

    #[test]
    fn gate_accepts_full_schema() {
        use std::collections::BTreeMap;
        let mut f1 = field("title", FieldTypeSerde::Scalar("String".into()));
        f1.required = vec!["**".into()];
        let mut f2 = field("status", FieldTypeSerde::Scalar("String".into()));
        f2.constraints = Some(Constraints {
            categories: Some(vec![toml::Value::String("draft".into())]),
            ..Default::default()
        });
        let f3 = field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        );
        let f4 = field(
            "meta",
            FieldTypeSerde::Object {
                object: BTreeMap::from([(
                    "author".into(),
                    FieldTypeSerde::Scalar("String".into()),
                )]),
            },
        );
        let out = dsl_to_canonical(&with_fields(vec![f1, f2, f3, f4]));
        assert!(validate_mdvs_schema(&out).is_ok(), "produced: {out}");
    }

    // ------------------------------------------------------------------------
    // Group C — gate rejects denied keywords and structures
    // ------------------------------------------------------------------------

    fn assert_rejects(schema: Value, expected_substr: &str) {
        let err =
            validate_mdvs_schema(&schema).expect_err(&format!("expected rejection for: {schema}"));
        assert!(
            err.contains(expected_substr),
            "expected error containing {expected_substr:?}, got: {err}"
        );
    }

    #[test]
    fn gate_rejects_one_of() {
        assert_rejects(
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}),
            "'oneOf' is not supported",
        );
    }

    #[test]
    fn gate_rejects_any_of() {
        assert_rejects(json!({"anyOf": []}), "'anyOf' is not supported");
    }

    #[test]
    fn gate_rejects_all_of() {
        assert_rejects(json!({"allOf": []}), "'allOf' is not supported");
    }

    #[test]
    fn gate_rejects_not() {
        assert_rejects(json!({"not": {}}), "'not' is not supported");
    }

    #[test]
    fn gate_rejects_if_then_else() {
        assert_rejects(json!({"if": {}, "then": {}}), "'if' is not supported");
    }

    #[test]
    fn gate_rejects_ref() {
        assert_rejects(json!({"$ref": "#/foo"}), "'$ref' is not supported");
    }

    #[test]
    fn gate_rejects_defs() {
        assert_rejects(json!({"$defs": {}}), "'$defs' is not supported");
    }

    #[test]
    fn gate_rejects_pattern_properties() {
        assert_rejects(
            json!({"patternProperties": {"^x": {}}}),
            "'patternProperties' is not supported",
        );
    }

    #[test]
    fn gate_rejects_prefix_items() {
        assert_rejects(json!({"prefixItems": []}), "'prefixItems' is not supported");
    }

    #[test]
    fn gate_rejects_format() {
        assert_rejects(json!({"format": "email"}), "'format' is not supported");
    }

    #[test]
    fn gate_rejects_unknown_root_keyword() {
        assert_rejects(
            json!({"madeUpKeyword": true}),
            "unknown keyword 'madeUpKeyword'",
        );
    }

    #[test]
    fn gate_rejects_x_mdvs_at_nested_location() {
        // x-mdvs allowed only at root or root-level property; not inside `items`.
        assert_rejects(
            json!({
                "type": "array",
                "items": {
                    "type": "string",
                    "x-mdvs": {"allowed": ["**"]}
                }
            }),
            "only valid at the schema root",
        );
    }

    #[test]
    fn gate_rejects_unknown_x_mdvs_subkey() {
        assert_rejects(
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "x-mdvs": {"thirdEye": true}
                    }
                }
            }),
            "unknown 'x-mdvs.thirdEye'",
        );
    }

    #[test]
    fn gate_rejects_property_subkey_at_root() {
        // 'allowed' is a property-level x-mdvs sub-key, not schema-level.
        assert_rejects(
            json!({
                "type": "object",
                "x-mdvs": {"allowed": ["**"]}
            }),
            "unknown 'x-mdvs.allowed'",
        );
    }

    #[test]
    fn gate_accepts_schema_level_x_mdvs() {
        // 'preprocess' and 'definitions' are valid at root.
        let schema = json!({
            "type": "object",
            "x-mdvs": {"preprocess": [], "definitions": {}}
        });
        assert!(validate_mdvs_schema(&schema).is_ok());
    }

    #[test]
    fn gate_accepts_metadata_keywords() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/foo",
            "title": "Foo",
            "description": "A foo schema",
            "type": "object"
        });
        assert!(validate_mdvs_schema(&schema).is_ok());
    }
}
