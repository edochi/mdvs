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
    "format",
    "$schema",
    "$id",
    "title",
    "description",
    "x-mdvs",
];

/// JSON Schema `format` values mdvs supports. Any other format value is
/// rejected by the gate with a "use pattern" hint.
const ALLOWED_FORMATS: &[&str] = &["date"];

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
/// - One entry under `properties` per `[[fields.field]]` (recursively nested
///   for dotted names — see below) and per `ignore` name (flat).
/// - Path-scoping carried as `x-mdvs.allowed` / `x-mdvs.required` on each
///   leaf property; intermediate Object nodes carry no `x-mdvs`.
/// - No root-level `required` array — requirement is path-scoped.
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

fn intermediate_object_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true,
        "properties": {},
    })
}

/// True if the value is the exact shape produced by [`intermediate_object_schema`]
/// (or its post-population state): an object schema with a `properties` map and
/// no `x-mdvs` metadata. Used by [`insert_at_path`] for the can-I-recurse check
/// and by [`canonical_to_dsl`] to distinguish structural Objects from leaf
/// Object schemas (Array-of-Object inner types and similar).
pub(crate) fn is_intermediate_object(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.get("type") == Some(&Value::String("object".into()))
        && obj.get("properties").map(Value::is_object).unwrap_or(false)
        && !obj.contains_key("x-mdvs")
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

// ============================================================================
// canonical_to_dsl — reverse translator
// ============================================================================

/// Output of [`canonical_to_dsl`]: the per-property fields plus any
/// empty-schema entries that map to the `[fields].ignore` list.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CanonicalImport {
    pub fields: Vec<crate::schema::config::TomlField>,
    pub ignore: Vec<String>,
}

/// Translate a canonical JSON Schema (presumed to have passed
/// [`validate_mdvs_schema`]) back into mdvs DSL fields.
///
/// Strict pattern matching: only the exact shapes [`dsl_to_canonical`]
/// produces are accepted. Anything else errors with a clear message
/// pointing at the property name.
///
/// **Dotted-name reconstruction** (TODO-0097 step 3): the nested
/// `properties` tree is walked with a path prefix. Intermediate Object
/// nodes (those produced by [`intermediate_object_schema`] — `{type:
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

    let mut fields: Vec<crate::schema::config::TomlField> = Vec::new();
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
    fields: &mut Vec<crate::schema::config::TomlField>,
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

fn field_from_subschema(
    name: &str,
    sub: &Map<String, Value>,
) -> Result<crate::schema::config::TomlField, String> {
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

    Ok(crate::schema::config::TomlField {
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

/// Inverse of [`build_x_mdvs`]: extract `allowed` / `required` / `preprocess`
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
    /// A property at any depth under `properties` / `items` /
    /// `additionalProperties`. After TODO-0097 step 3, mdvs has structural
    /// objects at arbitrary depth (dotted-name leaves create intermediates),
    /// so a single `Property` location replaces the earlier root-only
    /// distinction.
    Property,
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
                    walk(prop_schema, Location::Property)?;
                }
            }
            "items" => {
                walk(value, Location::Property)?;
            }
            // Allowed values: bool or schema. If schema, walk it.
            "additionalProperties" if value.is_object() => {
                walk(value, Location::Property)?;
            }
            "format" => {
                let format_str = value
                    .as_str()
                    .ok_or_else(|| "'format' must be a string".to_string())?;
                if !ALLOWED_FORMATS.contains(&format_str) {
                    return Err(format!(
                        "format '{format_str}' is not supported by mdvs — \
                         use 'pattern' for regex-based validation"
                    ));
                }
            }
            "x-mdvs" => {
                let xm = value
                    .as_object()
                    .ok_or_else(|| "'x-mdvs' must be an object".to_string())?;
                let allowed_subkeys = match location {
                    Location::Root => MDVS_KEYS_SCHEMA,
                    Location::Property => MDVS_KEYS_PROPERTY,
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
            preprocess: vec![],
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
    fn string_field_simple_strict() {
        // String fields emit strict `{"type": "string"}` — coercion is the
        // preprocessor's job, not the schema's.
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
    fn date_field_emits_string_with_format() {
        let toml = with_fields(vec![field(
            "birthday",
            FieldTypeSerde::Scalar("Date".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["birthday"],
            json!({"type": "string", "format": "date"})
        );
    }

    #[test]
    fn array_of_date_emits_items_format() {
        let toml = with_fields(vec![field(
            "milestones",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Date".into())),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["milestones"],
            json!({
                "type": "array",
                "items": {"type": "string", "format": "date"}
            })
        );
    }

    #[test]
    fn nullable_date_emits_union_with_format() {
        let mut f = field("birthday", FieldTypeSerde::Scalar("Date".into()));
        f.nullable = true;
        let toml = with_fields(vec![f]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["birthday"],
            json!({"type": ["string", "null"], "format": "date"})
        );
    }

    #[test]
    fn canonical_to_dsl_recognises_format_date() {
        // Build a canonical schema by hand, run canonical_to_dsl, expect Date.
        let schema = json!({
            "$schema": JSON_SCHEMA_DRAFT,
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "birthday": {"type": "string", "format": "date"}
            },
        });
        let imported = canonical_to_dsl(&schema).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "birthday");
        assert_eq!(fields[0].field_type, FieldTypeSerde::Scalar("Date".into()));
    }

    #[test]
    fn dsl_to_canonical_round_trip_with_date() {
        let toml = with_fields(vec![field(
            "birthday",
            FieldTypeSerde::Scalar("Date".into()),
        )]);
        let canonical = dsl_to_canonical(&toml);
        let imported = canonical_to_dsl(&canonical).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "birthday");
        assert_eq!(fields[0].field_type, FieldTypeSerde::Scalar("Date".into()));
    }

    #[test]
    fn nullable_string_field_emits_union_type() {
        // String + nullable=true: standard `["string", "null"]` union.
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({"type": ["string", "null"]})
        );
    }

    #[test]
    fn nullable_integer_field_emits_union_type() {
        // Non-String types follow the standard union-with-null pattern.
        let mut f = field("count", FieldTypeSerde::Scalar("Integer".into()));
        f.nullable = true;
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["count"],
            json!({"type": ["integer", "null"]})
        );
    }

    #[test]
    fn array_of_strings_field_strict() {
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
    fn array_of_object_emits_items_properties() {
        // After TODO-0097 step 3, Object as a top-level type is rejected by
        // MdvsToml::validate invariant 6 (top-level structure is expressed
        // via dotted-name leaves). Object survives only as an Array inner
        // type — and now the translator emits proper `items.properties`
        // children rather than the pre-Wave-C `additionalProperties: true`
        // placeholder.
        use std::collections::BTreeMap;
        let toml = with_fields(vec![field(
            "readings",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Object {
                    object: BTreeMap::from([
                        ("author".into(), FieldTypeSerde::Scalar("String".into())),
                        ("version".into(), FieldTypeSerde::Scalar("Integer".into())),
                    ]),
                }),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["readings"],
            json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "author": {"type": "string"},
                        "version": {"type": "integer"},
                    },
                }
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
    fn nullable_categorical_appends_null_to_enum() {
        // nullable=true + categories appends null to enum so null passes the
        // categorical check (matches the existing semantic).
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
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
            json!({"type": ["string", "null"], "enum": ["draft", "published", null]})
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
    fn length_constraint_emits_min_max_length() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(3),
            max_length: Some(64),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({"type": "string", "minLength": 3, "maxLength": 64})
        );
    }

    #[test]
    fn pattern_constraint_emits_pattern_keyword() {
        let mut f = field("slug", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            pattern: Some("^[a-z0-9-]+$".into()),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["slug"],
            json!({"type": "string", "pattern": "^[a-z0-9-]+$"})
        );
    }

    #[test]
    fn length_and_pattern_combined() {
        let mut f = field("token", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(8),
            max_length: Some(8),
            pattern: Some("^[A-Z]{8}$".into()),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["token"],
            json!({
                "type": "string",
                "minLength": 8,
                "maxLength": 8,
                "pattern": "^[A-Z]{8}$"
            })
        );
    }

    #[test]
    fn array_string_length_applies_to_items() {
        let mut f = field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        );
        f.constraints = Some(Constraints {
            min_length: Some(2),
            max_length: Some(20),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["tags"],
            json!({
                "type": "array",
                "items": {"type": "string", "minLength": 2, "maxLength": 20}
            })
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
    fn gate_rejects_unsupported_format() {
        assert_rejects(
            json!({"format": "email"}),
            "format 'email' is not supported",
        );
    }

    #[test]
    fn gate_rejects_date_time_format() {
        // Wave 3 adds date-time; until then, only `date` is allowed.
        assert_rejects(
            json!({"format": "date-time"}),
            "format 'date-time' is not supported",
        );
    }

    #[test]
    fn gate_rejects_non_string_format() {
        assert_rejects(json!({"format": 42}), "'format' must be a string");
    }

    #[test]
    fn gate_accepts_format_date() {
        assert!(validate_mdvs_schema(&json!({"format": "date"})).is_ok());
    }

    #[test]
    fn gate_rejects_unknown_root_keyword() {
        assert_rejects(
            json!({"madeUpKeyword": true}),
            "unknown keyword 'madeUpKeyword'",
        );
    }

    #[test]
    fn gate_accepts_x_mdvs_at_any_property_depth() {
        // After TODO-0097 step 3, dotted-name leaves can live at any depth.
        // The gate accepts property-level x-mdvs everywhere; only the root
        // location is gated to schema-level subkeys.
        assert!(
            validate_mdvs_schema(&json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "cal": {
                        "type": "object",
                        "additionalProperties": true,
                        "properties": {
                            "wavelength": {
                                "type": "number",
                                "x-mdvs": {"allowed": ["**"]}
                            }
                        }
                    }
                }
            }))
            .is_ok()
        );

        // Same allowance applies inside `items` (Array element schemas
        // are now valid property locations).
        assert!(
            validate_mdvs_schema(&json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "x-mdvs": {"allowed": ["**"]}
                        }
                    }
                }
            }))
            .is_ok()
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

    // ------------------------------------------------------------------------
    // canonical_to_dsl — reverse translator (TODO-0149 step 10)
    // ------------------------------------------------------------------------

    fn roundtrip(toml: MdvsToml) {
        let canonical = dsl_to_canonical(&toml);
        let import = canonical_to_dsl(&canonical).expect("reverse translation");
        // serde_json::Map preserves insertion order on round-trip but
        // dsl_to_canonical merges fields + ignore into a single `properties`
        // map; reading it back loses the original ordering distinction.
        // Compare as sorted sets.
        let mut got_fields = import.fields;
        got_fields.sort_by(|a, b| a.name.cmp(&b.name));
        let mut exp_fields = toml.fields.field.clone();
        exp_fields.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(got_fields, exp_fields, "fields mismatch");
        let mut got_ignore = import.ignore;
        got_ignore.sort();
        let mut exp_ignore = toml.fields.ignore.clone();
        exp_ignore.sort();
        assert_eq!(got_ignore, exp_ignore, "ignore mismatch");
    }

    #[test]
    fn roundtrip_string_field() {
        roundtrip(with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]));
    }

    #[test]
    fn roundtrip_integer_field() {
        roundtrip(with_fields(vec![field(
            "count",
            FieldTypeSerde::Scalar("Integer".into()),
        )]));
    }

    #[test]
    fn roundtrip_float_field() {
        roundtrip(with_fields(vec![field(
            "score",
            FieldTypeSerde::Scalar("Float".into()),
        )]));
    }

    #[test]
    fn roundtrip_boolean_field() {
        roundtrip(with_fields(vec![field(
            "draft",
            FieldTypeSerde::Scalar("Boolean".into()),
        )]));
    }

    #[test]
    fn roundtrip_nullable_string() {
        let mut f = field("note", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_nullable_integer() {
        let mut f = field("count", FieldTypeSerde::Scalar("Integer".into()));
        f.nullable = true;
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_array_of_strings() {
        roundtrip(with_fields(vec![field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        )]));
    }

    #[test]
    fn roundtrip_categories_constraint() {
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_nullable_plus_enum_strips_appended_null() {
        // dsl_to_canonical appends null to the enum when nullable=true;
        // canonical_to_dsl must strip it back.
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_range_constraint() {
        let mut f = field("rating", FieldTypeSerde::Scalar("Integer".into()));
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(5)),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_length_and_pattern() {
        let mut f = field("slug", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(3),
            max_length: Some(64),
            pattern: Some("^[a-z0-9-]+$".into()),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_preserves_preprocess() {
        use crate::preprocess::ValueStage;
        let mut f = field("funding", FieldTypeSerde::Scalar("String".into()));
        f.preprocess = vec![ValueStage::CoerceToString];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn dsl_to_canonical_emits_preprocess_in_x_mdvs() {
        use crate::preprocess::ValueStage;
        let mut f = field("score", FieldTypeSerde::Scalar("Float".into()));
        f.preprocess = vec![ValueStage::WidenIntToFloat];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["score"]["x-mdvs"]["preprocess"],
            json!(["widen-int-to-float"])
        );
    }

    #[test]
    fn roundtrip_path_scoping() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.allowed = vec!["blog/**".into(), "notes/**".into()];
        f.required = vec!["blog/**".into()];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_ignore_list() {
        let mut t = empty_toml();
        t.fields.ignore = vec!["internal_id".into(), "draft_meta".into()];
        roundtrip(t);
    }

    #[test]
    fn canonical_to_dsl_rejects_missing_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": {"enum": ["a", "b"]}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("missing 'type'"), "got: {err}");
    }

    #[test]
    fn canonical_to_dsl_rejects_invalid_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": {"type": "integer"},
                "y": {"type": "foobar"}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("unsupported type"), "got: {err}");
    }

    #[test]
    fn canonical_to_dsl_rejects_null_in_non_nullable_enum() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["draft", null]}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("not nullable"), "got: {err}");
    }

    // ========================================================================
    // TODO-0097 step 3: dotted-name flattening
    // ========================================================================

    #[test]
    fn dsl_to_canonical_single_dotted_creates_intermediate() {
        let toml = with_fields(vec![field(
            "calibration.baseline.wavelength",
            FieldTypeSerde::Scalar("Float".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        // Outer intermediate
        assert_eq!(out["properties"]["calibration"]["type"], "object");
        assert_eq!(
            out["properties"]["calibration"]["additionalProperties"],
            true
        );
        assert!(out["properties"]["calibration"].get("x-mdvs").is_none());
        // Inner intermediate
        assert_eq!(
            out["properties"]["calibration"]["properties"]["baseline"]["type"],
            "object"
        );
        // Leaf
        assert_eq!(
            out["properties"]["calibration"]["properties"]["baseline"]["properties"]["wavelength"]
                ["type"],
            "number"
        );
    }

    #[test]
    fn dsl_to_canonical_siblings_share_intermediate() {
        let toml = with_fields(vec![
            field("cal.x", FieldTypeSerde::Scalar("Float".into())),
            field("cal.y", FieldTypeSerde::Scalar("Float".into())),
        ]);
        let out = dsl_to_canonical(&toml);
        let cal_props = out["properties"]["cal"]["properties"].as_object().unwrap();
        assert!(cal_props.contains_key("x"));
        assert!(cal_props.contains_key("y"));
        assert_eq!(cal_props.len(), 2);
    }

    #[test]
    fn dsl_to_canonical_mixed_flat_and_dotted() {
        let toml = with_fields(vec![
            field("title", FieldTypeSerde::Scalar("String".into())),
            field("cal.wave", FieldTypeSerde::Scalar("Float".into())),
        ]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["title"]["type"], "string");
        assert_eq!(
            out["properties"]["cal"]["properties"]["wave"]["type"],
            "number"
        );
    }

    #[test]
    fn dsl_to_canonical_leaf_x_mdvs_lives_at_leaf_not_intermediate() {
        let mut f = field("cal.baseline.wave", FieldTypeSerde::Scalar("Float".into()));
        f.allowed = vec!["projects/alpha/**".into()];
        f.required = vec!["projects/alpha/**".into()];
        let toml = with_fields(vec![f]);
        let out = dsl_to_canonical(&toml);

        // Intermediates have no x-mdvs
        assert!(out["properties"]["cal"].get("x-mdvs").is_none());
        assert!(
            out["properties"]["cal"]["properties"]["baseline"]
                .get("x-mdvs")
                .is_none()
        );

        // Leaf carries x-mdvs
        let leaf_x =
            &out["properties"]["cal"]["properties"]["baseline"]["properties"]["wave"]["x-mdvs"];
        assert_eq!(leaf_x["allowed"], json!(["projects/alpha/**"]));
        assert_eq!(leaf_x["required"], json!(["projects/alpha/**"]));
    }

    #[test]
    fn canonical_to_dsl_walks_nested_into_dotted_names() {
        let schema = json!({
            "$schema": JSON_SCHEMA_DRAFT,
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "title": {"type": "string"},
                "cal": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "baseline": {
                            "type": "object",
                            "additionalProperties": true,
                            "properties": {
                                "wavelength": {"type": "number"},
                                "intensity": {"type": "number"}
                            }
                        }
                    }
                }
            }
        });
        let imported = canonical_to_dsl(&schema).unwrap();
        let names: Vec<&str> = imported.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"title"));
        assert!(names.contains(&"cal.baseline.wavelength"));
        assert!(names.contains(&"cal.baseline.intensity"));
        // Intermediates do NOT become TomlFields.
        assert!(!names.contains(&"cal"));
        assert!(!names.contains(&"cal.baseline"));
    }

    #[test]
    fn roundtrip_dotted_leaf() {
        let f = field(
            "calibration.baseline.wavelength",
            FieldTypeSerde::Scalar("Float".into()),
        );
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_dotted_with_x_mdvs() {
        use crate::preprocess::ValueStage;
        let mut f = field("cal.wave", FieldTypeSerde::Scalar("Float".into()));
        f.allowed = vec!["projects/alpha/**".into()];
        f.required = vec!["projects/alpha/**".into()];
        f.preprocess = vec![ValueStage::WidenIntToFloat];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_mixed_flat_and_dotted() {
        let toml = with_fields(vec![
            field("title", FieldTypeSerde::Scalar("String".into())),
            field("cal.x", FieldTypeSerde::Scalar("Float".into())),
            field("cal.y", FieldTypeSerde::Scalar("Float".into())),
            field("meta.author", FieldTypeSerde::Scalar("String".into())),
        ]);
        roundtrip(toml);
    }

    #[test]
    fn roundtrip_array_of_object_inner_shape_preserved() {
        use std::collections::BTreeMap;
        let toml = with_fields(vec![field(
            "readings",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Object {
                    object: BTreeMap::from([
                        ("time".into(), FieldTypeSerde::Scalar("String".into())),
                        ("value".into(), FieldTypeSerde::Scalar("Float".into())),
                    ]),
                }),
            },
        )]);
        roundtrip(toml);
    }

    #[test]
    fn canonical_to_dsl_rejects_empty_schema_at_nested_depth() {
        // An empty schema is only valid at the top level (signals an
        // ignored name). Nested empty schemas indicate a malformed input.
        let schema = json!({
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "cal": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "broken": {}
                    }
                }
            }
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(
            err.contains("empty schema is not allowed at nested depth"),
            "got: {err}"
        );
    }
}
