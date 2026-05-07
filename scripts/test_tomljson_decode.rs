#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde_json = "1"
//! toml = "1"
//! ```
//!
//! Spike for the TOML → JSON direction of `tomljson`.
//!
//! In mdvs's primary use case (parsing user-authored JSON Schema TOML),
//! this is the dominant direction. The encode-side (`test_tomljson_writer.rs`)
//! is symmetric in concept but carries different edge cases.
//!
//! Goals:
//!   - Exercise every entry in the TOML→JSON policy table.
//!   - Confirm the policy is implementable as a small tree-walker (no surprises).
//!   - Surface any decisions that need explicit documentation.
//!
//! Run: `rust-script scripts/test_tomljson_decode.rs`

use serde_json::{json, Value as Json};
use toml::Value as Toml;

const DEFAULT_NULL_PLACEHOLDER: &str = "__null__";
const ROOT_KEY: &str = "__root__";

#[derive(Debug)]
struct Error(String);

// ============================================================================
// TOML → JSON tree walker
// ============================================================================

fn toml_to_json(v: &Toml, placeholder: &str) -> Result<Json, Error> {
    match v {
        // Placeholder string → JSON null (must precede the general String arm)
        Toml::String(s) if s == placeholder => Ok(Json::Null),
        Toml::String(s) => Ok(Json::String(s.clone())),

        Toml::Integer(i) => Ok(Json::Number((*i).into())),

        Toml::Float(f) => {
            // TOML allows +inf, -inf, nan; JSON does not. Error explicitly
            // rather than silently dropping or corrupting.
            if f.is_nan() {
                return Err(Error(
                    "TOML float NaN cannot be represented in JSON; \
                     JSON Schema also forbids it (use omission to mean 'no constraint')".into(),
                ));
            }
            if f.is_infinite() {
                let sign = if *f > 0.0 { "+" } else { "-" };
                return Err(Error(format!(
                    "TOML float {sign}inf cannot be represented in JSON; \
                     omit `maximum`/`minimum` to indicate 'no bound'"
                )));
            }
            Ok(serde_json::Number::from_f64(*f)
                .map(Json::Number)
                .expect("finite float must convert"))
        }

        Toml::Boolean(b) => Ok(Json::Bool(*b)),

        // All four TOML datetime variants → canonical RFC 3339 string.
        // JSON Schema represents dates/times as strings with `format: date|time|date-time`.
        Toml::Datetime(dt) => Ok(Json::String(dt.to_string())),

        Toml::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for item in arr {
                out.push(toml_to_json(item, placeholder)?);
            }
            Ok(Json::Array(out))
        }

        Toml::Table(t) => {
            let mut obj = serde_json::Map::with_capacity(t.len());
            for (k, v) in t {
                obj.insert(k.clone(), toml_to_json(v, placeholder)?);
            }
            Ok(Json::Object(obj))
        }
    }
}

/// Top-level decoder. Parses TOML, applies placeholder substitution, unwraps __root__.
fn decode_with(s: &str, placeholder: &str) -> Result<Json, Error> {
    let parsed: Toml = toml::from_str(s).map_err(|e| Error(e.to_string()))?;
    let json = toml_to_json(&parsed, placeholder)?;

    // Unwrap __root__ if present (table with exactly one key __root__).
    if let Json::Object(ref obj) = json {
        if obj.len() == 1 {
            if let Some(v) = obj.get(ROOT_KEY) {
                return Ok(v.clone());
            }
        }
    }
    Ok(json)
}

fn decode(s: &str) -> Result<Json, Error> {
    decode_with(s, DEFAULT_NULL_PLACEHOLDER)
}

// ============================================================================
// Test driver
// ============================================================================

fn run(num: usize, name: &str, toml_str: &str, expected: Json) {
    println!("--- {}. {} ---", num, name);
    println!("input:  {}", toml_str.trim().replace('\n', "\n        "));
    let actual = decode(toml_str).unwrap_or_else(|e| panic!("decode failed: {}", e.0));
    println!("output: {}", actual);
    assert_eq!(actual, expected, "mismatch on test {} ({})", num, name);
    println!("  {}. {}  ✓\n", num, name);
}

fn run_should_fail(num: usize, name: &str, toml_str: &str) {
    println!("--- {}. {} (expected decode failure) ---", num, name);
    println!("input:  {}", toml_str.trim().replace('\n', "\n        "));
    match decode(toml_str) {
        Ok(j) => panic!("expected failure on test {} ({}); got: {}", num, name, j),
        Err(e) => {
            println!("error:  {}", e.0);
            println!("  {}. {}  ✓\n", num, name);
        }
    }
}

fn main() {
    println!("=== TOML → JSON decode spike ===\n");

    // ─── Standard scalars ───────────────────────────────────────────────────

    run(1, "string", r#"x = "hello""#, json!({ "x": "hello" }));

    run(2, "integer (positive)", r#"x = 42"#, json!({ "x": 42 }));

    run(3, "integer (negative)", r#"x = -42"#, json!({ "x": -42 }));

    run(
        4,
        "i64::MAX boundary",
        r#"x = 9223372036854775807"#,
        json!({ "x": 9223372036854775807_i64 }),
    );

    run(
        5,
        "i64::MIN boundary",
        r#"x = -9223372036854775808"#,
        json!({ "x": -9223372036854775808_i64 }),
    );

    run(6, "float (regular)", r#"x = 3.14"#, json!({ "x": 3.14 }));

    run(7, "boolean true", r#"x = true"#, json!({ "x": true }));
    run(8, "boolean false", r#"x = false"#, json!({ "x": false }));

    // ─── Datetime variants (all four → JSON string) ─────────────────────────

    run(
        9,
        "TOML local date → string",
        r#"x = 2026-05-04"#,
        json!({ "x": "2026-05-04" }),
    );

    run(
        10,
        "TOML local time → string",
        r#"x = 09:30:00"#,
        json!({ "x": "09:30:00" }),
    );

    run(
        11,
        "TOML local datetime → string",
        r#"x = 2026-05-04T09:30:00"#,
        json!({ "x": "2026-05-04T09:30:00" }),
    );

    run(
        12,
        "TOML offset datetime → string",
        r#"x = 2026-05-04T09:30:00Z"#,
        json!({ "x": "2026-05-04T09:30:00Z" }),
    );

    run(
        13,
        "TOML offset datetime with +offset → string",
        r#"x = 2026-05-04T09:30:00+02:00"#,
        json!({ "x": "2026-05-04T09:30:00+02:00" }),
    );

    // ─── Placeholder substitution ───────────────────────────────────────────

    run(
        14,
        "placeholder at scalar position → null",
        r#"default = "__null__""#,
        json!({ "default": null }),
    );

    run(
        15,
        "placeholder inside an array → null",
        r#"enum = ["a", "b", "__null__"]"#,
        json!({ "enum": ["a", "b", null] }),
    );

    run(
        16,
        "placeholder inside a nested object → null",
        "[default]\ncolor = \"__null__\"\n",
        json!({ "default": { "color": null } }),
    );

    // ─── Root unwrapping ────────────────────────────────────────────────────

    run(17, "__root__ true", r#"__root__ = true"#, json!(true));
    run(18, "__root__ false", r#"__root__ = false"#, json!(false));
    run(19, "__root__ array", r#"__root__ = [1, 2, 3]"#, json!([1, 2, 3]));
    run(
        20,
        "__root__ scalar string",
        r#"__root__ = "hello""#,
        json!("hello"),
    );

    // ─── Absent keys ────────────────────────────────────────────────────────

    run(
        21,
        "absent key (no special handling)",
        r#"x = 1"#,
        json!({ "x": 1 }),
    );
    // Note: `y` is absent in TOML, absent in JSON. There is no "null y" intermediate.

    // ─── Arrays + tables (recursive) ────────────────────────────────────────

    run(
        22,
        "heterogeneous array",
        r#"enum = [1, "two", true, "__null__"]"#,
        json!({ "enum": [1, "two", true, null] }),
    );

    run(
        23,
        "nested tables",
        "[a.b]\nc = 1\n",
        json!({ "a": { "b": { "c": 1 } } }),
    );

    run(
        24,
        "inline table",
        r#"x = { a = 1, b = "two" }"#,
        json!({ "x": { "a": 1, "b": "two" } }),
    );

    run(
        25,
        "array of tables",
        "[[items]]\nname = \"first\"\n[[items]]\nname = \"second\"\n",
        json!({ "items": [{ "name": "first" }, { "name": "second" }] }),
    );

    // ─── Custom placeholder via API ─────────────────────────────────────────

    println!("--- 26. custom placeholder (placeholder = \"@@NULL@@\") ---");
    let custom_toml = r#"
"$tomljson-null" = "@@NULL@@"
default = "@@NULL@@"
"#;
    let parsed: Toml = toml::from_str(custom_toml).unwrap();
    // In production this scan + remove of $tomljson-null happens in the
    // standalone API entry point; here we hand-roll it for the spike.
    let placeholder = if let Toml::Table(ref t) = parsed {
        if let Some(Toml::String(s)) = t.get("$tomljson-null") {
            s.clone()
        } else {
            DEFAULT_NULL_PLACEHOLDER.to_string()
        }
    } else {
        DEFAULT_NULL_PLACEHOLDER.to_string()
    };
    let mut json = toml_to_json(&parsed, &placeholder).unwrap();
    if let Json::Object(ref mut obj) = json {
        obj.remove("$tomljson-null");
    }
    println!("input:  {}", custom_toml.trim().replace('\n', "\n        "));
    println!("output: {}", json);
    assert_eq!(json, json!({ "default": null }));
    println!("  26. custom placeholder  ✓\n");

    // ─── Failure cases ──────────────────────────────────────────────────────

    run_should_fail(27, "TOML +inf errors", r#"x = inf"#);
    run_should_fail(28, "TOML -inf errors", r#"x = -inf"#);
    run_should_fail(29, "TOML nan errors", r#"x = nan"#);

    run_should_fail(
        30,
        "+inf inside array errors",
        r#"x = [1.0, inf, 2.0]"#,
    );

    run_should_fail(
        31,
        "nan in nested table errors",
        "[constraints]\nupper = nan\n",
    );

    println!("=== TOML → JSON decode spike: all expected outcomes verified ===");
}
