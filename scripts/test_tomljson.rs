#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde_json = "1"
//! toml = "1"
//! ```
//!
//! Prototype for TODO-0149 Wave A: validate the lossless JSON ↔ TOML encoding
//! rules before committing to them in the `tomljson` crate. JSON Schema is the
//! motivating use case but the translator itself is JSON-Schema-agnostic.
//!
//! Run: `rust-script scripts/test_tomljson.rs`

use serde_json::{json, Value as Json};
use toml::Value as Toml;

// ============================================================================
// Encoding rules
// ============================================================================
//
// JSON and TOML data models nearly overlap. Two real gaps:
//   1. JSON has `null`; TOML does not.
//   2. JSON Schema allows a *whole* schema to be `true` or `false`; TOML root
//      must be a table.
//
// Conventions chosen here:
//   - `null` → a string placeholder (default "__null__"), occupying the slot
//     wherever a null appears. Customizable per-document via the
//     `$tomljson-null` root directive. Encoder errors if any real string
//     value collides with the placeholder.
//   - Non-object roots wrapped as `__root__ = <value>`.
//   - All other values translate point-for-point. Number fidelity preserved
//     (integers stay integers, floats stay floats).

const DEFAULT_NULL_PLACEHOLDER: &str = "__null__";
const NULL_DIRECTIVE: &str = "$tomljson-null";
const ROOT_KEY: &str = "__root__";

#[derive(Debug)]
struct EncodeError(String);

/// Walk the JSON tree and assert encodability:
/// - no string equals the null placeholder (collision)
/// - no integer exceeds TOML's signed 64-bit range (i64::MAX)
fn assert_encodable(v: &Json, placeholder: &str) -> Result<(), EncodeError> {
    match v {
        Json::Null | Json::Bool(_) => Ok(()),
        Json::Number(n) => {
            // serde_json::Number stores either i64, u64, or f64. If as_i64()
            // is None and is_u64() is true, the value is in (i64::MAX, u64::MAX]
            // and TOML's signed 64-bit Integer cannot hold it.
            if n.as_i64().is_none() && n.is_u64() {
                Err(EncodeError(format!(
                    "schema contains integer {} which exceeds TOML's signed 64-bit range \
                     (i64::MAX = 9223372036854775807); TOML cannot represent this value losslessly",
                    n
                )))
            } else {
                Ok(())
            }
        }
        Json::String(s) => {
            if s == placeholder {
                Err(EncodeError(format!(
                    "schema contains string {:?} which collides with the null placeholder; \
                     pick a different placeholder via $tomljson-null",
                    s
                )))
            } else {
                Ok(())
            }
        }
        Json::Array(arr) => arr.iter().try_for_each(|x| assert_encodable(x, placeholder)),
        Json::Object(obj) => obj.values().try_for_each(|x| assert_encodable(x, placeholder)),
    }
}

fn json_to_toml(v: &Json, placeholder: &str) -> Toml {
    match v {
        Json::Null => Toml::String(placeholder.to_string()),
        Json::Bool(b) => Toml::Boolean(*b),
        Json::Number(n) => {
            // assert_encodable has already rejected u64 > i64::MAX, so the only
            // possibilities here are i64-fit ints or f64 floats.
            if let Some(i) = n.as_i64() {
                Toml::Integer(i)
            } else if let Some(f) = n.as_f64() {
                Toml::Float(f)
            } else {
                unreachable!("number {:?} should have been rejected by assert_encodable", n);
            }
        }
        Json::String(s) => Toml::String(s.clone()),
        Json::Array(arr) => Toml::Array(arr.iter().map(|x| json_to_toml(x, placeholder)).collect()),
        Json::Object(obj) => {
            let mut t = toml::map::Map::new();
            for (k, v) in obj {
                t.insert(k.clone(), json_to_toml(v, placeholder));
            }
            Toml::Table(t)
        }
    }
}

fn toml_to_json(v: &Toml, placeholder: &str) -> Json {
    match v {
        Toml::String(s) if s == placeholder => Json::Null,
        Toml::String(s) => Json::String(s.clone()),
        Toml::Integer(i) => Json::Number((*i).into()),
        Toml::Float(f) => serde_json::Number::from_f64(*f)
            .map(Json::Number)
            .unwrap_or(Json::Null),
        Toml::Boolean(b) => Json::Bool(*b),
        Toml::Datetime(dt) => Json::String(dt.to_string()),
        Toml::Array(arr) => Json::Array(arr.iter().map(|x| toml_to_json(x, placeholder)).collect()),
        Toml::Table(t) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in t {
                obj.insert(k.clone(), toml_to_json(v, placeholder));
            }
            Json::Object(obj)
        }
    }
}

fn encode(v: &Json) -> Result<String, EncodeError> {
    encode_with(v, DEFAULT_NULL_PLACEHOLDER)
}

fn encode_with(v: &Json, placeholder: &str) -> Result<String, EncodeError> {
    assert_encodable(v, placeholder)?;
    let toml_val = json_to_toml(v, placeholder);
    let needs_directive = placeholder != DEFAULT_NULL_PLACEHOLDER;

    let mut root = match toml_val {
        Toml::Table(t) => t,
        other => {
            // Non-object root: wrap under __root__.
            let mut m = toml::map::Map::new();
            m.insert(ROOT_KEY.into(), other);
            m
        }
    };

    if needs_directive {
        // Directive at the top of the file (alphabetical ordering will sort
        // `$tomljson-null` before bare keys due to `$` < ASCII letters).
        root.insert(NULL_DIRECTIVE.into(), Toml::String(placeholder.into()));
    }

    Ok(toml::to_string(&root).map_err(|e| EncodeError(e.to_string()))?)
}

fn decode(s: &str) -> Json {
    let mut parsed: Toml = toml::from_str(s).unwrap();
    let mut placeholder = DEFAULT_NULL_PLACEHOLDER.to_string();

    // Strip and apply the directive if present.
    if let Toml::Table(ref mut t) = parsed {
        if let Some(Toml::String(custom)) = t.remove(NULL_DIRECTIVE) {
            placeholder = custom;
        }
    }

    if let Toml::Table(ref t) = parsed {
        if t.len() == 1 && t.contains_key(ROOT_KEY) {
            return toml_to_json(&t[ROOT_KEY], &placeholder);
        }
    }
    toml_to_json(&parsed, &placeholder)
}

// ============================================================================
// Test driver
// ============================================================================

fn run(num: usize, name: &str, schema: Json) {
    println!("--- {}. {} ---", num, name);
    let toml_str = encode(&schema).unwrap_or_else(|e| panic!("encode failed: {}", e.0));
    print!("{}", toml_str);
    if !toml_str.ends_with('\n') {
        println!();
    }
    let back = decode(&toml_str);
    assert_eq!(back, schema, "roundtrip mismatch on test {} ({})", num, name);
    println!("  {}. {}  ✓\n", num, name);
}

fn run_with(num: usize, name: &str, schema: Json, placeholder: &str) {
    println!("--- {}. {} (placeholder = {:?}) ---", num, name, placeholder);
    let toml_str = encode_with(&schema, placeholder)
        .unwrap_or_else(|e| panic!("encode failed: {}", e.0));
    print!("{}", toml_str);
    if !toml_str.ends_with('\n') {
        println!();
    }
    let back = decode(&toml_str);
    assert_eq!(back, schema, "roundtrip mismatch on test {} ({})", num, name);
    println!("  {}. {}  ✓\n", num, name);
}

fn run_should_fail(num: usize, name: &str, schema: Json) {
    println!("--- {}. {} (expected encode failure) ---", num, name);
    match encode(&schema) {
        Ok(_) => panic!("expected encode failure on test {} ({})", num, name),
        Err(e) => {
            println!("  error: {}", e.0);
            println!("  {}. {}  ✓\n", num, name);
        }
    }
}

fn main() {
    println!("=== tomljson encoding prototype ===\n");

    // 1. Trivial scalar type
    run(1, "type: string", json!({ "type": "string" }));

    // 2. Object with properties + required
    run(
        2,
        "object with properties + required",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer" }
            },
            "required": ["name"]
        }),
    );

    // 3. Numeric constraints
    run(
        3,
        "numeric constraints",
        json!({
            "type": "number",
            "minimum": 0,
            "maximum": 100,
            "exclusiveMinimum": -1
        }),
    );

    // 4. String constraints
    run(
        4,
        "string constraints",
        json!({
            "type": "string",
            "minLength": 1,
            "maxLength": 64,
            "pattern": "^[a-z][a-z0-9_-]*$"
        }),
    );

    // 5. Homogeneous enum
    run(
        5,
        "enum (homogeneous strings)",
        json!({ "enum": ["draft", "published", "archived"] }),
    );

    // 6. Heterogeneous enum
    run(
        6,
        "enum (mixed types)",
        json!({ "enum": [1, "two", true] }),
    );

    // 7. Enum with null
    run(
        7,
        "enum with null",
        json!({ "enum": ["a", "b", null] }),
    );

    // 7a. const: null (scalar position)
    run(
        7,
        "const = null",
        json!({ "const": null }),
    );

    // 7b. default: null (scalar position)
    run(
        7,
        "default = null with nullable type",
        json!({ "type": ["string", "null"], "default": null }),
    );

    // 7c. null nested deep inside default
    run(
        7,
        "default contains null inside an object/array",
        json!({
            "type": "object",
            "default": {
                "color": null,
                "tags": ["a", null, "b"]
            }
        }),
    );

    // 8. Array with items
    run(
        8,
        "array with items",
        json!({
            "type": "array",
            "items": { "type": "string", "minLength": 1 }
        }),
    );

    // 9. prefixItems (tuple form)
    run(
        9,
        "prefixItems (tuple)",
        json!({
            "type": "array",
            "prefixItems": [
                { "type": "string" },
                { "type": "integer" }
            ],
            "items": false
        }),
    );

    // 10. oneOf composition (array-of-tables)
    run(
        10,
        "oneOf composition",
        json!({
            "oneOf": [
                { "type": "string" },
                { "type": "integer", "minimum": 0 }
            ]
        }),
    );

    // 11. anyOf + allOf + not
    run(
        11,
        "anyOf + allOf + not",
        json!({
            "allOf": [
                { "type": "object" },
                { "required": ["id"] }
            ],
            "anyOf": [
                { "properties": { "kind": { "const": "a" } } },
                { "properties": { "kind": { "const": "b" } } }
            ],
            "not": { "required": ["deprecated"] }
        }),
    );

    // 12. $defs + $ref
    run(
        12,
        "$defs + $ref",
        json!({
            "type": "object",
            "properties": {
                "billing": { "$ref": "#/$defs/address" },
                "shipping": { "$ref": "#/$defs/address" }
            },
            "$defs": {
                "address": {
                    "type": "object",
                    "properties": {
                        "street": { "type": "string" },
                        "city": { "type": "string" }
                    },
                    "required": ["street", "city"]
                }
            }
        }),
    );

    // 13. if / then / else
    run(
        13,
        "if/then/else",
        json!({
            "type": "object",
            "properties": { "kind": { "type": "string" } },
            "if": { "properties": { "kind": { "const": "premium" } } },
            "then": { "required": ["billing"] },
            "else": { "required": ["email"] }
        }),
    );

    // 14. Deep nesting
    run(
        14,
        "deeply nested properties",
        json!({
            "type": "object",
            "properties": {
                "user": {
                    "type": "object",
                    "properties": {
                        "address": {
                            "type": "object",
                            "properties": {
                                "city": { "type": "string" }
                            }
                        }
                    }
                }
            }
        }),
    );

    // 15. x-mdvs-* extensions
    run(
        15,
        "x-mdvs-* extensions",
        json!({
            "type": "string",
            "x-mdvs-allowed": ["**/*.md"],
            "x-mdvs-required": ["posts/**/*.md"]
        }),
    );

    // 16. Boolean schema (always-valid)
    run(16, "boolean schema true", json!(true));

    // 17. Number fidelity in enum
    run(
        17,
        "number fidelity (int vs float)",
        json!({ "enum": [1, 1.0, 1.5] }),
    );

    // 18. Realistic composite — also pin the canonical TOML form
    let composite = json!({
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
    });
    run(18, "realistic composite", composite.clone());

    // Also: canonical hand-written TOML must parse to the same JSON.
    println!("--- 18b. canonical TOML decodes to same JSON ---");
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
    let decoded = decode(canonical);
    assert_eq!(decoded, composite, "canonical TOML did not decode to composite");
    println!("  18b. canonical TOML decodes to same JSON  ✓\n");

    // 19. Custom placeholder via $tomljson-null directive
    run_with(
        19,
        "custom placeholder",
        json!({
            "type": ["string", "null"],
            "default": null,
            "enum": ["a", "b", null]
        }),
        "@@NULL@@",
    );

    // 20. Collision: schema string equals default placeholder → encode fails
    run_should_fail(
        20,
        "collision with default placeholder",
        json!({ "enum": ["draft", "__null__", "archived"] }),
    );

    // 20a. Strings that look like TOML scalar literals roundtrip as strings.
    run(
        20,
        "string values shaped like TOML literals",
        json!({
            "enum": ["true", "false", "42", "1.5", "2026-05-04", "inf", "nan"]
        }),
    );

    // 20b. Hand-written unquoted TOML datetime decodes to a JSON string.
    println!("--- 20b. unquoted TOML datetime → JSON string ---");
    let unquoted = r#"
type = "string"
default = 2026-05-04
"#;
    let decoded = decode(unquoted);
    assert_eq!(
        decoded,
        json!({ "type": "string", "default": "2026-05-04" }),
        "unquoted datetime should decode to a JSON string"
    );
    println!("  decoded: {}", decoded);
    println!("  20b. unquoted TOML datetime → JSON string  ✓\n");

    // 20c. Empty root schema.
    run(20, "empty root schema {}", json!({}));

    // 20d. i64::MAX roundtrips fine.
    run(
        20,
        "i64::MAX boundary",
        json!({ "const": 9223372036854775807i64 }),
    );

    // 20e. i64::MIN roundtrips fine.
    run(
        20,
        "i64::MIN boundary",
        json!({ "const": -9223372036854775808i64 }),
    );

    // 20f. u64 just past i64::MAX → encode errors per TOML spec.
    run_should_fail(
        20,
        "integer past i64::MAX errors",
        json!({ "const": 9223372036854775808u64 }),
    );

    // 20g. u64::MAX → encode errors.
    run_should_fail(
        20,
        "u64::MAX errors",
        json!({ "const": 18446744073709551615u64 }),
    );

    // 20h. Float precision boundary (toml-lang #44 concern).
    // f64::to_string uses Ryū which guarantees shortest round-trip; verify.
    run(
        20,
        "f64 precision roundtrip",
        json!({
            "examples": [
                0.1 + 0.2,                  // 0.30000000000000004
                f64::MIN_POSITIVE,          // ~2.2e-308 (subnormal boundary)
                f64::MAX,                   // ~1.8e308
                f64::EPSILON,               // ~2.2e-16
                std::f64::consts::PI,
                -1e-300_f64,
                1e300_f64,
            ]
        }),
    );

    // 20i. Unquoted TOML local time → JSON string.
    println!("--- 20i. unquoted TOML local time → JSON string ---");
    let local_time = "default = 09:30:00\n";
    let decoded = decode(local_time);
    assert_eq!(
        decoded,
        json!({ "default": "09:30:00" }),
        "unquoted local time should decode to a JSON string"
    );
    println!("  decoded: {}", decoded);
    println!("  20i. unquoted TOML local time → JSON string  ✓\n");

    // 20j. Unicode in string values (JSON Schema descriptions, examples).
    run(
        20,
        "unicode in strings",
        json!({
            "description": "café ☕ — 日本語 — 🚀",
            "default": "🎉"
        }),
    );

    // 20k. Empty string as a value.
    run(
        20,
        "empty string value",
        json!({ "default": "", "const": "" }),
    );

    // 20l. String with embedded newlines.
    run(
        20,
        "string with embedded newlines",
        json!({ "description": "line one\nline two\nline three" }),
    );

    // 21. Collision avoided by choosing a different placeholder
    println!("--- 21. collision avoided with custom placeholder ---");
    let collision_schema = json!({ "enum": ["draft", "__null__", null] });
    let toml_str = encode_with(&collision_schema, "@@NULL@@")
        .unwrap_or_else(|e| panic!("encode failed: {}", e.0));
    print!("{}", toml_str);
    if !toml_str.ends_with('\n') {
        println!();
    }
    let back = decode(&toml_str);
    assert_eq!(back, collision_schema);
    println!("  21. collision avoided with custom placeholder  ✓\n");

    println!("=== All tests passed ===");
}
