//! `dsl_to_canonical` — translate an [`MdvsToml`] DSL into a canonical
//! JSON Schema 2020-12 document.
//!
//! Output shape:
//! - `type: "object"`, `additionalProperties: true` at the root (per-file
//!   overlay synthesis tightens this — see TODO-0149 step 13).
//! - One entry under `properties` per `[[fields.field]]` (recursively
//!   nested for dotted names) and per `ignore` name (flat).
//! - Path-scoping carried as `x-mdvs.allowed` / `x-mdvs.required` on
//!   each leaf property; intermediate Object nodes carry no `x-mdvs`.
//! - No root-level `required` array — requirement is path-scoped.

use crate::discover::field_type::FieldType;
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::constraints::Constraints;
use serde_json::{Map, Value, json};

use super::is_intermediate_object;

/// JSON Schema 2020-12 `$schema` URI.
pub(super) const JSON_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Translate an `MdvsToml` DSL into a canonical JSON Schema 2020-12 document.
///
/// **Dotted-name flattening** (TODO-0097 step 3): a `[[fields.field]]` whose
/// `name` contains `.` is placed at the corresponding nested path. For
/// example `calibration.baseline.wavelength` lands at
/// `root.properties.calibration.properties.baseline.properties.wavelength`.
/// Intermediate Object nodes (`calibration`, `calibration.baseline` in the
/// example) are auto-created as `{type: "object", additionalProperties:
/// true, properties: {...}}`. Shape conflicts among field names
/// (declaring `foo` as a leaf and `foo.bar` as a different leaf) are caught
/// by [`MdvsToml::validate`]'s invariant 8 before this function runs.
pub(crate) fn dsl_to_canonical(toml: &MdvsToml) -> Value {
    let mut properties: Map<String, Value> = Map::new();

    for field in &toml.fields.field {
        let segments: Vec<&str> = field.name.split('.').collect();
        let leaf = field_to_subschema(field);
        insert_at_path(&mut properties, &segments, leaf);
    }
    for ignored in &toml.fields.ignore {
        // Empty schema = always-passes. Step 13's overlay may further constrain.
        // Ignored names are treated as flat (no nested-leaf semantics).
        properties.insert(ignored.clone(), json!({}));
    }

    json!({
        "$schema": JSON_SCHEMA_DRAFT,
        "type": "object",
        "properties": Value::Object(properties),
        "additionalProperties": true,
    })
}

/// Place `leaf` at the dotted path described by `segments`. Creates
/// intermediate `{type: "object", additionalProperties: true, properties: {}}`
/// nodes as needed.
///
/// Shape conflicts (a leaf already exists where an intermediate is needed,
/// or vice versa) are caught upstream by `MdvsToml::validate`'s invariant 8.
/// If they reach this function, the later insertion silently replaces the
/// earlier — but in well-validated input this never happens.
fn insert_at_path(properties: &mut Map<String, Value>, segments: &[&str], leaf: Value) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };

    if rest.is_empty() {
        properties.insert(first.to_string(), leaf);
        return;
    }

    let entry = properties
        .entry(first.to_string())
        .or_insert_with(intermediate_object_schema);

    // Defensive: if validation skipped, an entry might not be an
    // intermediate. Replace rather than panic — we trust validate to
    // surface the conflict to the user before they reach this path.
    if !is_intermediate_object(entry) {
        *entry = intermediate_object_schema();
    }

    // The intermediate_object_schema() call above guarantees `properties`
    // exists as an object. If a future edit breaks that invariant, silently
    // skip rather than panic — the schema will be incomplete but the
    // program won't crash.
    if let Some(inner_props) = entry.get_mut("properties").and_then(Value::as_object_mut) {
        insert_at_path(inner_props, rest, leaf);
    }
}

pub(super) fn intermediate_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {},
    })
}

fn field_to_subschema(field: &TomlField) -> Value {
    let ft = match FieldType::try_from(&field.field_type) {
        Ok(ft) => ft,
        // Unparseable types are caught by `MdvsToml::validate()` before this
        // function is reachable. If we get here, fall through to an empty schema.
        Err(_) => return json!({}),
    };

    let mut subschema = type_subschema(&ft, field.nullable, field.constraints.as_ref());

    let x_mdvs = build_x_mdvs(field);
    if !x_mdvs.is_empty()
        && let Some(map) = subschema.as_object_mut()
    {
        // type_subschema always returns an object; the guard above guards
        // against a future edit changing that.
        map.insert("x-mdvs".to_string(), Value::Object(x_mdvs));
    }

    subschema
}

/// Produce the JSON Schema for a field type, applying `nullable` and any
/// `Constraints` at the appropriate level (scalar fields apply constraints to
/// the scalar; array fields apply constraints to `items`).
///
/// Strict types throughout: type-coercion is the preprocessor pipeline's job
/// (see `crate::preprocess`), not the schema's. A field declared `String`
/// rejects non-string values unless its `preprocess` array includes
/// `coerce_to_string`.
fn type_subschema(ft: &FieldType, nullable: bool, constraints: Option<&Constraints>) -> Value {
    match ft {
        FieldType::Boolean => scalar_subschema("boolean", nullable, None),
        FieldType::Integer => scalar_subschema("integer", nullable, constraints),
        FieldType::Float => scalar_subschema("number", nullable, constraints),
        FieldType::String => scalar_subschema("string", nullable, constraints),
        FieldType::Date => {
            // `Date` is encoded as a JSON string with `format: date` (RFC 3339
            // full-date). The `jsonschema` crate validates the format when the
            // Validator is built with `should_validate_formats(true)`.
            let mut obj = Map::new();
            insert_type(&mut obj, "string", nullable);
            obj.insert("format".into(), Value::String("date".into()));
            apply_constraints(&mut obj, nullable, constraints);
            Value::Object(obj)
        }
        FieldType::DateTime => {
            // `DateTime` is encoded as a JSON string with `format: date-time`
            // (RFC 3339 datetime).
            let mut obj = Map::new();
            insert_type(&mut obj, "string", nullable);
            obj.insert("format".into(), Value::String("date-time".into()));
            apply_constraints(&mut obj, nullable, constraints);
            Value::Object(obj)
        }
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
        FieldType::Object(fields) => {
            // After TODO-0097 step 3, top-level Object fields are rejected
            // by `MdvsToml::validate`'s invariant 6. This arm is reached
            // only as a recursive call for `Array(Object{...})` inner types
            // (and any future `Object` inside `Array`-style composites).
            //
            // Per the scope decision, structured array elements stay inline:
            // we emit proper `properties` for the children rather than the
            // permissive `additionalProperties: true` placeholder used pre-Wave-C.
            let mut props = Map::new();
            for (name, inner_ft) in fields {
                // Inner Object children carry no nullability or constraints
                // of their own (those live on the outer `[[fields.field]]`'s
                // type subschema, not on its grandchildren).
                props.insert(name.clone(), type_subschema(inner_ft, false, None));
            }
            let mut obj = Map::new();
            insert_type(&mut obj, "object", nullable);
            obj.insert("additionalProperties".into(), Value::Bool(true));
            obj.insert("properties".into(), Value::Object(props));
            Value::Object(obj)
        }
    }
}

fn scalar_subschema(ty: &str, nullable: bool, constraints: Option<&Constraints>) -> Value {
    let mut obj = Map::new();
    insert_type(&mut obj, ty, nullable);
    apply_constraints(&mut obj, nullable, constraints);
    Value::Object(obj)
}

fn apply_constraints(
    obj: &mut Map<String, Value>,
    nullable: bool,
    constraints: Option<&Constraints>,
) {
    if let Some(c) = constraints {
        if let Some(cats) = &c.categories {
            // For nullable fields, append null to the enum list so null
            // passes the categorical check (matching the existing semantic
            // where null on nullable+categorical skips the enum violation).
            let mut enum_values: Vec<Value> = cats.iter().filter_map(toml_to_json).collect();
            if nullable {
                enum_values.push(Value::Null);
            }
            obj.insert("enum".into(), Value::Array(enum_values));
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
        if let Some(min) = c.min_length {
            obj.insert("minLength".into(), json!(min));
        }
        if let Some(max) = c.max_length {
            obj.insert("maxLength".into(), json!(max));
        }
        if let Some(pat) = &c.pattern {
            obj.insert("pattern".into(), Value::String(pat.clone()));
        }
    }
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

fn build_x_mdvs(field: &TomlField) -> Map<String, Value> {
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
    if !field.preprocess.is_empty() {
        let stages: Vec<Value> = field
            .preprocess
            .iter()
            .map(|s| Value::String(s.to_string()))
            .collect();
        out.insert("preprocess".into(), Value::Array(stages));
    }
    out
}

fn is_default_allowed(allowed: &[String]) -> bool {
    allowed.len() == 1 && allowed[0] == "**"
}
