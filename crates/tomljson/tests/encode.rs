//! Encode-direction integration tests.
//!
//! Lifted from `scripts/test_tomljson_writer.rs`. Each test encodes a JSON
//! value and verifies round-trip equivalence by parsing the TOML output back
//! through `toml::from_str` and substituting the placeholder string with
//! `Json::Null`.
//!
//! When the deserialize side lands (TODO-0149 Wave A step 5), the inline
//! `decode_for_test` helper here will be replaced with `tomljson::from_str`.

use serde_json::{Value as Json, json};
use tomljson::{DEFAULT_NULL_PLACEHOLDER, Error};

const ROOT_KEY: &str = "__root__";

// ============================================================================
// Test-only decode helper — round-trips encode output back to JSON.
//
// Production decode lives in step 5; this helper is intentionally minimal
// and only handles what the encode tests produce.
// ============================================================================

fn decode_for_test(s: &str) -> Json {
    let parsed: toml::Value = toml::from_str(s).expect("encoded TOML must parse");
    let json = toml_to_json(&parsed, DEFAULT_NULL_PLACEHOLDER);

    // Unwrap __root__ if present.
    if let Json::Object(ref obj) = json
        && obj.len() == 1
        && let Some(v) = obj.get(ROOT_KEY)
    {
        return v.clone();
    }
    json
}

fn toml_to_json(v: &toml::Value, placeholder: &str) -> Json {
    match v {
        toml::Value::String(s) if s == placeholder => Json::Null,
        toml::Value::String(s) => Json::String(s.clone()),
        toml::Value::Integer(i) => Json::Number((*i).into()),
        toml::Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        toml::Value::Boolean(b) => Json::Bool(*b),
        toml::Value::Datetime(dt) => Json::String(dt.to_string()),
        toml::Value::Array(arr) => {
            Json::Array(arr.iter().map(|x| toml_to_json(x, placeholder)).collect())
        }
        toml::Value::Table(t) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in t {
                obj.insert(k.clone(), toml_to_json(v, placeholder));
            }
            Json::Object(obj)
        }
    }
}

fn assert_roundtrips(value: Json) {
    let toml_str = tomljson::to_string(&value).expect("encode succeeded");
    let back = decode_for_test(&toml_str);
    assert_eq!(back, value, "round-trip mismatch\n--- toml ---\n{toml_str}");
}

// ============================================================================
// Cases 1-15 — successful encode + round-trip
// ============================================================================

#[test]
fn case_01_trivial() {
    assert_roundtrips(json!({ "type": "string" }));
}

#[test]
fn case_02_scalars_and_required_array() {
    assert_roundtrips(json!({
        "type": "object",
        "minLength": 1,
        "maxLength": 64,
        "required": ["name", "age"]
    }));
}

#[test]
fn case_03_enum_with_null() {
    assert_roundtrips(json!({ "enum": ["draft", "published", null] }));
}

#[test]
fn case_04_enum_with_mixed_types() {
    assert_roundtrips(json!({ "enum": [1, "two", true, null] }));
}

#[test]
fn case_05_nested_properties() {
    assert_roundtrips(json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "age": { "type": "integer", "minimum": 0 }
        }
    }));
}

#[test]
fn case_06_deeply_nested() {
    assert_roundtrips(json!({
        "properties": {
            "user": {
                "properties": {
                    "address": {
                        "properties": {
                            "city": { "type": "string" }
                        }
                    }
                }
            }
        }
    }));
}

#[test]
fn case_07_defs_and_ref_quoted_keys() {
    assert_roundtrips(json!({
        "$defs": {
            "address": { "type": "object" }
        },
        "properties": {
            "billing": { "$ref": "#/$defs/address" }
        }
    }));
}

#[test]
fn case_08_one_of_as_array_of_tables() {
    assert_roundtrips(json!({
        "oneOf": [
            { "type": "string" },
            { "type": "integer", "minimum": 0 }
        ]
    }));
}

#[test]
fn case_09_top_level_boolean_schema() {
    assert_roundtrips(json!(true));
}

#[test]
fn case_10_top_level_array_root_wrap() {
    assert_roundtrips(json!([1, 2, 3]));
}

#[test]
fn case_11_f64_precision() {
    assert_roundtrips(json!({
        "examples": [0.1 + 0.2, std::f64::consts::PI]
    }));
}

#[test]
fn case_12_unicode_and_embedded_newlines() {
    assert_roundtrips(json!({
        "description": "café ☕\nline two — 日本語",
        "default": ""
    }));
}

#[test]
fn case_13_i64_boundaries() {
    assert_roundtrips(json!({ "examples": [i64::MAX, i64::MIN] }));
}

#[test]
fn case_14_default_null() {
    assert_roundtrips(json!({
        "type": ["string", "null"],
        "default": null
    }));
}

#[test]
fn case_15_empty_schema() {
    assert_roundtrips(json!({}));
}

// ============================================================================
// Cases 16-18 — encode failures
// ============================================================================

#[test]
fn case_16_u64_above_i64_max_errors() {
    let value = json!({ "const": 9_223_372_036_854_775_808_u64 });
    match tomljson::to_string(&value) {
        Err(Error::IntegerOutOfRange { value: v, .. }) => {
            assert_eq!(v, 9_223_372_036_854_775_808);
        }
        Err(other) => panic!("expected IntegerOutOfRange, got: {other:?}"),
        Ok(s) => panic!("expected error, got TOML: {s}"),
    }
}

#[test]
fn case_17_u64_max_errors() {
    let value = json!({ "const": u64::MAX });
    match tomljson::to_string(&value) {
        Err(Error::IntegerOutOfRange { value: v, .. }) => {
            assert_eq!(v, u64::MAX);
        }
        Err(other) => panic!("expected IntegerOutOfRange, got: {other:?}"),
        Ok(s) => panic!("expected error, got TOML: {s}"),
    }
}

#[test]
fn case_18_string_collision_with_placeholder() {
    let value = json!({ "enum": ["a", "__null__"] });
    match tomljson::to_string(&value) {
        Err(Error::PlaceholderCollision { placeholder, .. }) => {
            assert_eq!(placeholder, "__null__");
        }
        Err(other) => panic!("expected PlaceholderCollision, got: {other:?}"),
        Ok(s) => panic!("expected error, got TOML: {s}"),
    }
}
