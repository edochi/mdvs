//! Encode-direction integration tests.
//!
//! Each test encodes a JSON value and verifies round-trip equivalence
//! through `tomljson::from_str`.

use serde_json::{Value as Json, json};
use tomljson::{Error, TomlJsonOptions};

fn assert_roundtrips(value: Json) {
    let toml_str = tomljson::to_string(&value).expect("encode succeeded");
    let back = tomljson::from_str(&toml_str).expect("decode succeeded");
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

// ============================================================================
// Cases 19-21 — root placeholder collision + custom root placeholder
// ============================================================================

#[test]
fn case_19_root_key_collision_errors() {
    // Input has a top-level key `__root__` as legitimate data. With default
    // options, the encoder/decoder pair would silently strip it on round-trip.
    let value = json!({ "__root__": "data" });
    match tomljson::to_string(&value) {
        Err(Error::RootKeyCollision { placeholder }) => {
            assert_eq!(placeholder, "__root__");
        }
        Err(other) => panic!("expected RootKeyCollision, got: {other:?}"),
        Ok(s) => panic!("expected error, got TOML: {s}"),
    }
}

#[test]
fn case_20_custom_root_placeholder_avoids_collision() {
    // Same input but with a different root_placeholder configured; the round-trip
    // works because nothing in the data collides with the new wrapper key.
    let value = json!({ "__root__": "data" });
    let options = TomlJsonOptions {
        root_placeholder: "@@WRAP@@".to_string(),
        ..TomlJsonOptions::default()
    };
    let toml_str = tomljson::to_string_with_options(&value, &options).expect("encode succeeded");
    let back = tomljson::from_str_with_options(&toml_str, &options).expect("decode succeeded");
    assert_eq!(back, value, "round-trip mismatch\n--- toml ---\n{toml_str}");
}

#[test]
fn case_21_custom_root_placeholder_wraps_non_table() {
    // The custom root placeholder is also used to wrap non-table roots.
    let value = json!([1, 2, 3]);
    let options = TomlJsonOptions {
        root_placeholder: "@@WRAP@@".to_string(),
        ..TomlJsonOptions::default()
    };
    let toml_str = tomljson::to_string_with_options(&value, &options).expect("encode succeeded");
    // toml_writer quotes keys that aren't bare-key-valid (only [A-Za-z0-9_-]).
    assert!(
        toml_str.contains("@@WRAP@@"),
        "expected custom wrapper key in output, got: {toml_str}"
    );
    let back = tomljson::from_str_with_options(&toml_str, &options).expect("decode succeeded");
    assert_eq!(back, value);
}
