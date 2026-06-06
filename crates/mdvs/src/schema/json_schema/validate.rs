//! `validate_mdvs_schema` — gate that rejects JSON Schema documents using
//! keywords outside the mdvs subset.
//!
//! The gate is the boundary the `init --from-jsonschema` and `check
//! --jsonschema` flows cross when ingesting external schemas. Anything
//! beyond the [`ALLOW_LIST`] (or in the curated [`HARD_REJECT`] list with
//! a specific user-facing reason) is refused before `canonical_to_dsl`
//! sees it.

use serde_json::Value;

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
const ALLOWED_FORMATS: &[&str] = &["date", "date-time"];

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

/// Recognized `x-mdvs` sub-keys at the schema (root) level.
const MDVS_KEYS_SCHEMA: &[&str] = &["preprocess", "definitions"];

/// Recognized `x-mdvs` sub-keys at the property level.
const MDVS_KEYS_PROPERTY: &[&str] = &["allowed", "required", "preprocess"];

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
