//! Encoding-rules baseline tests.
//!
//! Lifted from `scripts/test_tomljson.rs` — the broadest spike, focused on
//! JSON-Schema-shaped inputs and end-to-end round-trip. Cases here exercise
//! the encoder + decoder together on realistic schemas; the encoder-internals
//! and decoder-internals tests live in `encode.rs` and `decode.rs`.

use serde_json::{Value as Json, json};

fn assert_roundtrips(value: Json) {
    let toml_str = tomljson::to_string(&value).expect("encode succeeded");
    let back = tomljson::from_str(&toml_str).expect("decode succeeded");
    assert_eq!(back, value, "round-trip mismatch\n--- toml ---\n{toml_str}");
}

/// Assert that hand-authored TOML (i.e., not produced by our encoder)
/// decodes to the expected JSON. Useful for verifying the schema is
/// human-friendly to author by hand, not just reachable via encode.
fn assert_decodes_to(toml_str: &str, expected: Json) {
    let back = tomljson::from_str(toml_str).expect("decode succeeded");
    assert_eq!(
        back, expected,
        "hand-written TOML decoded to wrong JSON\n--- toml ---\n{toml_str}"
    );
}

// ============================================================================
// Constraint shapes
// ============================================================================

#[test]
fn numeric_constraints() {
    assert_roundtrips(json!({
        "type": "number",
        "minimum": 0,
        "maximum": 100,
        "exclusiveMinimum": -1
    }));
}

#[test]
fn string_constraints_with_pattern() {
    assert_roundtrips(json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 64,
        "pattern": "^[a-z][a-z0-9_-]*$"
    }));
}

#[test]
fn homogeneous_string_enum() {
    assert_roundtrips(json!({
        "enum": ["draft", "published", "archived"]
    }));
}

#[test]
fn mixed_types_enum_no_null() {
    assert_roundtrips(json!({
        "enum": [1, "two", true]
    }));
}

#[test]
fn const_null_at_scalar_position() {
    assert_roundtrips(json!({ "const": null }));
}

#[test]
fn default_with_null_nested_in_object_and_array() {
    assert_roundtrips(json!({
        "type": "object",
        "default": {
            "color": null,
            "tags": ["a", null, "b"]
        }
    }));
}

#[test]
fn array_with_items_schema() {
    assert_roundtrips(json!({
        "type": "array",
        "items": { "type": "string", "minLength": 1 }
    }));
}

#[test]
fn prefix_items_tuple_form() {
    assert_roundtrips(json!({
        "type": "array",
        "prefixItems": [
            { "type": "string" },
            { "type": "integer" }
        ],
        "items": false
    }));
}

// ============================================================================
// Composition keywords
// ============================================================================

#[test]
fn any_of_all_of_not_combined() {
    assert_roundtrips(json!({
        "allOf": [
            { "type": "object" },
            { "required": ["id"] }
        ],
        "anyOf": [
            { "properties": { "kind": { "const": "a" } } },
            { "properties": { "kind": { "const": "b" } } }
        ],
        "not": { "required": ["deprecated"] }
    }));
}

#[test]
fn if_then_else() {
    assert_roundtrips(json!({
        "type": "object",
        "properties": { "kind": { "type": "string" } },
        "if": { "properties": { "kind": { "const": "premium" } } },
        "then": { "required": ["billing"] },
        "else": { "required": ["email"] }
    }));
}

// ============================================================================
// Extension keywords (x-mdvs-* family — tests quoted-key emission)
// ============================================================================

#[test]
fn x_mdvs_extensions_flat_form() {
    assert_roundtrips(json!({
        "type": "string",
        "x-mdvs-allowed": ["**/*.md"],
        "x-mdvs-required": ["posts/**/*.md"]
    }));
}

// ============================================================================
// Number fidelity
// ============================================================================

#[test]
fn number_fidelity_int_vs_float_preserved() {
    // serde_json distinguishes Number-as-i64 from Number-as-f64.
    // The encoder must emit `1` and `1.0` and `1.5` as their original kinds,
    // and the decoder must preserve the distinction.
    assert_roundtrips(json!({ "enum": [1, 1.0, 1.5] }));
}

// ============================================================================
// Realistic composite — the killer end-to-end test
// ============================================================================

#[test]
fn realistic_composite_schema() {
    assert_roundtrips(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://mdvs.dev/schema/post.json",
        "type": "object",
        "properties": {
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "status": { "enum": ["draft", "published", "archived"] },
            "author": { "$ref": "#/$defs/person" },
            "tags": {
                "type": "array",
                "items": { "type": "string", "pattern": "^[a-z][a-z0-9-]*$" },
                "minItems": 0,
                "maxItems": 16
            },
            "rating": {
                "oneOf": [
                    { "type": "number", "minimum": 0, "maximum": 5 },
                    { "type": "null" }
                ]
            }
        },
        "required": ["title", "status"],
        "$defs": {
            "person": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" },
                    "email": { "type": "string", "pattern": "^[^@]+@[^@]+$" }
                },
                "required": ["name"]
            }
        },
        "x-mdvs-allowed": ["posts/**/*.md"],
        "x-mdvs-required": ["posts/**/*.md"]
    }));
}

#[test]
fn canonical_hand_written_toml_decodes_to_composite() {
    // A human-friendly TOML form of the same schema as `realistic_composite_schema`.
    // This verifies the encoding is something a human would actually want to write
    // by hand, not just encoder output.
    let canonical = r##"
"$schema" = "https://json-schema.org/draft/2020-12/schema"
"$id" = "https://mdvs.dev/schema/post.json"
type = "object"
required = ["title", "status"]
"x-mdvs-allowed" = ["posts/**/*.md"]
"x-mdvs-required" = ["posts/**/*.md"]

[properties.title]
type = "string"
minLength = 1
maxLength = 200

[properties.status]
enum = ["draft", "published", "archived"]

[properties.author]
"$ref" = "#/$defs/person"

[properties.tags]
type = "array"
minItems = 0
maxItems = 16

[properties.tags.items]
type = "string"
pattern = "^[a-z][a-z0-9-]*$"

[properties.rating]

[[properties.rating.oneOf]]
type = "number"
minimum = 0
maximum = 5

[[properties.rating.oneOf]]
type = "null"

["$defs".person]
type = "object"
required = ["name"]

["$defs".person.properties.name]
type = "string"

["$defs".person.properties.email]
type = "string"
pattern = "^[^@]+@[^@]+$"
"##;

    let expected = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://mdvs.dev/schema/post.json",
        "type": "object",
        "required": ["title", "status"],
        "x-mdvs-allowed": ["posts/**/*.md"],
        "x-mdvs-required": ["posts/**/*.md"],
        "properties": {
            "title": { "type": "string", "minLength": 1, "maxLength": 200 },
            "status": { "enum": ["draft", "published", "archived"] },
            "author": { "$ref": "#/$defs/person" },
            "tags": {
                "type": "array",
                "minItems": 0,
                "maxItems": 16,
                "items": { "type": "string", "pattern": "^[a-z][a-z0-9-]*$" }
            },
            "rating": {
                "oneOf": [
                    { "type": "number", "minimum": 0, "maximum": 5 },
                    { "type": "null" }
                ]
            }
        },
        "$defs": {
            "person": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string" },
                    "email": { "type": "string", "pattern": "^[^@]+@[^@]+$" }
                }
            }
        }
    });

    assert_decodes_to(canonical, expected);
}

// ============================================================================
// String values shaped like TOML literals — must stay as strings
// ============================================================================

#[test]
fn strings_shaped_like_toml_literals_stay_strings() {
    // None of these should decode as their TOML-typed counterparts.
    assert_roundtrips(json!({
        "enum": ["true", "false", "42", "1.5", "2026-05-04", "inf", "nan"]
    }));
}

// ============================================================================
// Hand-authored TOML using TOML's native datetime syntax (unquoted)
// ============================================================================

#[test]
fn unquoted_toml_date_decodes_to_string() {
    let toml_str = r#"
type = "string"
default = 2026-05-04
"#;
    assert_decodes_to(
        toml_str,
        json!({ "type": "string", "default": "2026-05-04" }),
    );
}

#[test]
fn unquoted_toml_local_time_decodes_to_string() {
    let toml_str = "default = 09:30:00\n";
    assert_decodes_to(toml_str, json!({ "default": "09:30:00" }));
}

// ============================================================================
// Edge-case strings
// ============================================================================

#[test]
fn empty_string_value() {
    assert_roundtrips(json!({ "default": "", "const": "" }));
}
