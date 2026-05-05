#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde_json = "1"
//! toml = "1"
//! toml_writer = "1"
//! ```
//!
//! Spike for tomljson Implementation E: emit TOML directly via `toml_writer`,
//! bypassing serde's Serializer state machine entirely.
//!
//! The dispatcher walks a `serde_json::Value` and calls into `toml_writer`'s
//! low-level primitives. Where TOML can't represent JSON's data model
//! (`null`, top-level non-table, `u64 > i64::MAX`), the dispatcher applies
//! the placeholder / wrapper / error strategy itself before calling
//! `toml_writer`.
//!
//! Goals of the spike:
//!   1. Confirm `toml_writer`'s API actually supports the operations we need.
//!   2. Validate that a small dispatcher (~200 LOC) can produce well-formed
//!      TOML for representative JSON Schema-shaped inputs.
//!   3. Verify the emitted TOML parses back via `toml::from_str` to the
//!      expected structure (round-trip up to placeholder substitution).
//!   4. Surface any TOML-formatting decisions (inline vs. section, key
//!      quoting, mixed-array handling) before committing to crate work.
//!
//! Run: `rust-script scripts/test_tomljson_writer.rs`

use serde_json::{json, Value as Json};
use std::fmt::Write as _;
use toml_writer::TomlWrite;

const DEFAULT_NULL_PLACEHOLDER: &str = "__null__";
const ROOT_KEY: &str = "__root__";

#[derive(Debug)]
struct Error(String);

impl From<std::fmt::Error> for Error {
    fn from(e: std::fmt::Error) -> Self {
        Error(e.to_string())
    }
}

// ============================================================================
// Pre-flight check
// ============================================================================

fn assert_encodable(v: &Json, placeholder: &str) -> Result<(), Error> {
    match v {
        Json::Null | Json::Bool(_) => Ok(()),
        Json::Number(n) => {
            if n.as_i64().is_none() && n.is_u64() {
                return Err(Error(format!(
                    "integer {} exceeds TOML's signed 64-bit range \
                     (i64::MAX = 9223372036854775807); TOML cannot represent it losslessly",
                    n
                )));
            }
            Ok(())
        }
        Json::String(s) => {
            if s == placeholder {
                Err(Error(format!(
                    "string {:?} collides with the null placeholder; pick a different placeholder",
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

// ============================================================================
// Inline value emission (used for arrays and inline tables)
// ============================================================================

fn write_inline<W: TomlWrite>(w: &mut W, v: &Json, placeholder: &str) -> Result<(), Error> {
    match v {
        Json::Null => w.value(placeholder)?,
        Json::Bool(b) => w.value(*b)?,
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                w.value(i)?;
            } else if let Some(f) = n.as_f64() {
                w.value(f)?;
            } else {
                unreachable!("u64 overflow rejected in assert_encodable");
            }
        }
        Json::String(s) => w.value(s.as_str())?,
        Json::Array(arr) => {
            w.open_array()?;
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    w.val_sep()?;
                    w.space()?;
                }
                write_inline(w, item, placeholder)?;
            }
            w.close_array()?;
        }
        Json::Object(obj) => {
            // Inline table form: { k = v, k = v }
            w.open_inline_table()?;
            for (i, (k, val)) in obj.iter().enumerate() {
                if i > 0 {
                    w.val_sep()?;
                }
                w.space()?;
                w.key(k.as_str())?;
                w.space()?;
                w.keyval_sep()?;
                w.space()?;
                write_inline(w, val, placeholder)?;
            }
            if !obj.is_empty() {
                w.space()?;
            }
            w.close_inline_table()?;
        }
    }
    Ok(())
}

// ============================================================================
// Document-level emission (sections + scalars at the top of a table)
// ============================================================================

/// Returns true if the array is non-empty and every element is a JSON Object.
/// Such arrays are emitted as `[[path]]` array-of-tables sections.
fn is_array_of_tables(arr: &[Json]) -> bool {
    !arr.is_empty() && arr.iter().all(|v| matches!(v, Json::Object(_)))
}

fn write_table<W: TomlWrite>(
    w: &mut W,
    path: &[&str],
    obj: &serde_json::Map<String, Json>,
    placeholder: &str,
    is_first_section: &mut bool,
) -> Result<(), Error> {
    // Pass 1: emit inline keys (scalars, non-table-arrays, inline objects).
    // Pass 2: emit sub-tables and arrays-of-tables as their own sections.
    let mut sub_tables: Vec<(&str, &serde_json::Map<String, Json>)> = Vec::new();
    let mut sub_aots: Vec<(&str, &Vec<Json>)> = Vec::new();

    for (k, v) in obj {
        match v {
            Json::Object(child) => sub_tables.push((k, child)),
            Json::Array(arr) if is_array_of_tables(arr) => sub_aots.push((k, arr)),
            _ => {
                w.key(k.as_str())?;
                w.space()?;
                w.keyval_sep()?;
                w.space()?;
                write_inline(w, v, placeholder)?;
                w.newline()?;
            }
        }
    }

    // Sub-tables.
    for (k, child) in sub_tables {
        if *is_first_section {
            *is_first_section = false;
        } else {
            // Already had output; separate sections with a blank line.
        }
        w.newline()?;
        w.open_table_header()?;
        for p in path {
            w.key(*p)?;
            w.key_sep()?;
        }
        w.key(k)?;
        w.close_table_header()?;
        w.newline()?;

        let mut new_path: Vec<&str> = path.iter().copied().collect();
        new_path.push(k);
        write_table(w, &new_path, child, placeholder, is_first_section)?;
    }

    // Arrays of tables.
    for (k, arr) in sub_aots {
        for item in arr {
            let table = match item {
                Json::Object(t) => t,
                _ => unreachable!("is_array_of_tables guarantees Object"),
            };
            w.newline()?;
            w.open_array_of_tables_header()?;
            for p in path {
                w.key(*p)?;
                w.key_sep()?;
            }
            w.key(k)?;
            w.close_array_of_tables_header()?;
            w.newline()?;

            let mut new_path: Vec<&str> = path.iter().copied().collect();
            new_path.push(k);
            write_table(w, &new_path, table, placeholder, is_first_section)?;
        }
    }

    Ok(())
}

// ============================================================================
// Top-level encoder
// ============================================================================

fn encode_with(v: &Json, placeholder: &str) -> Result<String, Error> {
    assert_encodable(v, placeholder)?;
    let mut out = String::new();

    match v {
        Json::Object(obj) => {
            let mut first = true;
            write_table(&mut out, &[], obj, placeholder, &mut first)?;
        }
        other => {
            // Wrap non-table root under __root__.
            out.write_str(ROOT_KEY)?;
            out.write_str(" = ")?;
            write_inline(&mut out, other, placeholder)?;
            out.push('\n');
        }
    }

    Ok(out)
}

fn encode(v: &Json) -> Result<String, Error> {
    encode_with(v, DEFAULT_NULL_PLACEHOLDER)
}

// ============================================================================
// Decode (inverse, for round-trip validation)
//
// Uses toml::from_str to parse, then walks the resulting toml::Value
// substituting the placeholder string with serde_json::Value::Null.
// ============================================================================

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
        toml::Value::Array(arr) => Json::Array(arr.iter().map(|x| toml_to_json(x, placeholder)).collect()),
        toml::Value::Table(t) => {
            let mut obj = serde_json::Map::new();
            for (k, v) in t {
                obj.insert(k.clone(), toml_to_json(v, placeholder));
            }
            Json::Object(obj)
        }
    }
}

fn decode(s: &str) -> Result<Json, Error> {
    let parsed: toml::Value = toml::from_str(s).map_err(|e| Error(e.to_string()))?;
    let json = toml_to_json(&parsed, DEFAULT_NULL_PLACEHOLDER);

    // Unwrap __root__ if present.
    if let Json::Object(ref obj) = json {
        if obj.len() == 1 {
            if let Some(v) = obj.get(ROOT_KEY) {
                return Ok(v.clone());
            }
        }
    }
    Ok(json)
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
    let back = decode(&toml_str).unwrap_or_else(|e| panic!("decode failed: {}", e.0));
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
    println!("=== toml_writer dispatcher spike ===\n");

    run(1, "trivial", json!({ "type": "string" }));

    run(
        2,
        "scalars + required array",
        json!({
            "type": "object",
            "minLength": 1,
            "maxLength": 64,
            "required": ["name", "age"]
        }),
    );

    run(
        3,
        "enum with null",
        json!({ "enum": ["draft", "published", null] }),
    );

    run(
        4,
        "enum with mixed types",
        json!({ "enum": [1, "two", true, null] }),
    );

    run(
        5,
        "nested properties (sub-tables)",
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "age": { "type": "integer", "minimum": 0 }
            }
        }),
    );

    run(
        6,
        "deeply nested",
        json!({
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
        }),
    );

    run(
        7,
        "$defs + $ref (quoted keys)",
        json!({
            "$defs": {
                "address": { "type": "object" }
            },
            "properties": {
                "billing": { "$ref": "#/$defs/address" }
            }
        }),
    );

    run(
        8,
        "oneOf as array-of-tables",
        json!({
            "oneOf": [
                { "type": "string" },
                { "type": "integer", "minimum": 0 }
            ]
        }),
    );

    run(
        9,
        "top-level boolean schema",
        json!(true),
    );

    run(
        10,
        "top-level array (root wrap)",
        json!([1, 2, 3]),
    );

    run(
        11,
        "f64 precision",
        json!({ "examples": [0.1 + 0.2, std::f64::consts::PI] }),
    );

    run(
        12,
        "Unicode + embedded newlines",
        json!({
            "description": "café ☕\nline two — 日本語",
            "default": ""
        }),
    );

    run(
        13,
        "i64 boundaries",
        json!({ "examples": [i64::MAX, i64::MIN] }),
    );

    run(
        14,
        "default = null",
        json!({ "type": ["string", "null"], "default": null }),
    );

    run(
        15,
        "empty schema {}",
        json!({}),
    );

    run_should_fail(
        16,
        "u64 > i64::MAX errors",
        json!({ "const": 9223372036854775808u64 }),
    );

    run_should_fail(
        17,
        "u64::MAX errors",
        json!({ "const": 18446744073709551615u64 }),
    );

    run_should_fail(
        18,
        "string collision with placeholder",
        json!({ "enum": ["a", "__null__"] }),
    );

    println!("=== Spike complete: all expected outcomes verified ===");
}
