#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde_json = "1"
//! jsonschema = "0.46"
//! ```
//!
//! Spike for the `jsonschema::ErrorKind` → mdvs `ViolationKind` mapping
//! (TODO-0149, Open item #2).
//!
//! Goal: exercise every JSON Schema keyword that mdvs's Wave B translator
//! will emit, capture the resulting `ErrorKind` variant for each, and
//! produce a mapping table that the production translator can consume.
//!
//! mdvs's current `ViolationKind` enum:
//!   MissingRequired, WrongType, Disallowed,
//!   NullNotAllowed, InvalidCategory, OutOfRange
//!
//! Constraints mdvs will generate (Wave B + subsumed TODOs):
//!   type, required, additionalProperties, enum,
//!   minimum, exclusiveMinimum, maximum, exclusiveMaximum,
//!   minLength, maxLength, pattern,
//!   minItems, maxItems, uniqueItems
//!
//! Run: `rust-script scripts/test_violation_mapping.rs`

use jsonschema::error::ValidationErrorKind;
use serde_json::{json, Value as Json};

/// Minimal mdvs ViolationKind clone for the spike. Real one in `src/output.rs`.
#[derive(Debug, PartialEq, Eq, Clone)]
enum MdvsKind {
    MissingRequired,
    WrongType,
    Disallowed,
    NullNotAllowed,
    InvalidCategory,
    OutOfRange,
    /// Catch-all for jsonschema kinds we don't have a specific mdvs mapping for.
    /// Real impl would render these as `WrongType` with the keyword in `detail`.
    Unmapped(String),
}

/// The mapping function under test. This is the body that ships in
/// `crates/mdvs/src/schema/json_schema.rs` (Wave B).
fn map_error(instance: &Json, kind: &ValidationErrorKind) -> MdvsKind {
    use ValidationErrorKind as E;
    match kind {
        // Presence
        E::Required { .. } => MdvsKind::MissingRequired,
        E::AdditionalProperties { .. } => MdvsKind::Disallowed,

        // Type — distinguish "value is null + null forbidden" from "value is wrong shape".
        E::Type { .. } => {
            if instance.is_null() {
                MdvsKind::NullNotAllowed
            } else {
                MdvsKind::WrongType
            }
        }

        // Categories (enum constraint, including TODO-0006 subsumed)
        E::Enum { .. } | E::Constant { .. } => MdvsKind::InvalidCategory,

        // Numeric bounds (TODO-0008 subsumed)
        E::Minimum { .. }
        | E::Maximum { .. }
        | E::ExclusiveMinimum { .. }
        | E::ExclusiveMaximum { .. }
        | E::MultipleOf { .. } => MdvsKind::OutOfRange,

        // String length / pattern (TODO-0010, TODO-0145 subsumed)
        E::MinLength { .. }
        | E::MaxLength { .. } => MdvsKind::OutOfRange,
        E::Pattern { .. } => MdvsKind::WrongType,  // pattern mismatch ≈ wrong shape

        // Array bounds
        E::MinItems { .. }
        | E::MaxItems { .. }
        | E::UniqueItems { .. } => MdvsKind::OutOfRange,

        // Anything else — fallback to a marker so the test surfaces it.
        other => MdvsKind::Unmapped(format!("{other:?}").split_whitespace().next().unwrap_or("?").to_string()),
    }
}

// ============================================================================
// Test driver
// ============================================================================

fn run(num: usize, name: &str, schema: Json, instance: Json, expected: Vec<MdvsKind>) {
    println!("--- {}. {} ---", num, name);
    println!("schema:   {}", schema);
    println!("instance: {}", instance);

    let validator = jsonschema::validator_for(&schema).expect("schema must compile");
    let errors: Vec<_> = validator.iter_errors(&instance).collect();
    println!("raw errors:");
    for e in &errors {
        println!("  - kind={:?} path={}", e.kind(), e.instance_path());
    }

    // For each error, find the offending instance value to feed `map_error`.
    let actual: Vec<MdvsKind> = errors
        .iter()
        .map(|e| {
            let v = resolve_instance_path(&instance, &e.instance_path().to_string());
            map_error(v, e.kind())
        })
        .collect();

    println!("mapped:   {:?}", actual);
    println!("expected: {:?}", expected);
    assert_eq!(actual, expected, "mismatch on test {} ({})", num, name);
    println!("  {}. {}  ✓\n", num, name);
}

fn resolve_instance_path<'a>(root: &'a Json, path: &str) -> &'a Json {
    let mut cur = root;
    for seg in path.trim_start_matches('/').split('/').filter(|s| !s.is_empty()) {
        cur = match cur {
            Json::Object(m) => m.get(seg).unwrap_or(&Json::Null),
            Json::Array(a) => a.get(seg.parse::<usize>().unwrap_or(0)).unwrap_or(&Json::Null),
            _ => &Json::Null,
        };
    }
    cur
}

fn main() {
    println!("=== ValidationError → ViolationKind mapping spike ===\n");

    // ─── MissingRequired ────────────────────────────────────────────────────
    run(
        1, "missing required field",
        json!({ "type": "object", "required": ["title"], "properties": { "title": { "type": "string" } } }),
        json!({}),
        vec![MdvsKind::MissingRequired],
    );

    // ─── Disallowed (additionalProperties) ──────────────────────────────────
    run(
        2, "additional property rejected",
        json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        json!({ "rogue": 1 }),
        vec![MdvsKind::Disallowed],
    );

    // ─── WrongType ──────────────────────────────────────────────────────────
    run(
        3, "wrong scalar type (string expected, got integer)",
        json!({ "type": "string" }),
        json!(42),
        vec![MdvsKind::WrongType],
    );

    run(
        4, "wrong type — object expected, got array",
        json!({ "type": "object" }),
        json!([1, 2]),
        vec![MdvsKind::WrongType],
    );

    // ─── NullNotAllowed (null value vs non-null type) ───────────────────────
    run(
        5, "null on non-nullable string field",
        json!({ "type": "string" }),
        json!(null),
        vec![MdvsKind::NullNotAllowed],
    );

    // Sanity: nullable: true (encoded as type union with null) accepts null.
    println!("--- 6. nullable string accepts null (sanity, no errors) ---");
    let schema = json!({ "type": ["string", "null"] });
    let validator = jsonschema::validator_for(&schema).unwrap();
    let errs: Vec<_> = validator.iter_errors(&json!(null)).collect();
    assert!(errs.is_empty(), "nullable union must accept null, got: {:?}",
            errs.iter().map(|e| format!("{:?}", e.kind())).collect::<Vec<_>>());
    println!("  6. nullable string accepts null  ✓\n");

    // ─── InvalidCategory (enum) ─────────────────────────────────────────────
    run(
        7, "value not in enum",
        json!({ "enum": ["draft", "published", "archived"] }),
        json!("scheduled"),
        vec![MdvsKind::InvalidCategory],
    );

    // ─── InvalidCategory (const) ────────────────────────────────────────────
    run(
        8, "const mismatch",
        json!({ "const": "fixed" }),
        json!("other"),
        vec![MdvsKind::InvalidCategory],
    );

    // ─── OutOfRange (numeric bounds, TODO-0008) ─────────────────────────────
    run(
        9, "below minimum",
        json!({ "type": "integer", "minimum": 0 }),
        json!(-1),
        vec![MdvsKind::OutOfRange],
    );

    run(
        10, "above maximum",
        json!({ "type": "integer", "maximum": 100 }),
        json!(150),
        vec![MdvsKind::OutOfRange],
    );

    run(
        11, "exclusiveMinimum violated",
        json!({ "type": "number", "exclusiveMinimum": 0 }),
        json!(0),
        vec![MdvsKind::OutOfRange],
    );

    run(
        12, "exclusiveMaximum violated",
        json!({ "type": "number", "exclusiveMaximum": 1 }),
        json!(1),
        vec![MdvsKind::OutOfRange],
    );

    run(
        13, "multipleOf violated",
        json!({ "type": "integer", "multipleOf": 5 }),
        json!(7),
        vec![MdvsKind::OutOfRange],
    );

    // ─── OutOfRange (string length, TODO-0010) ──────────────────────────────
    run(
        14, "minLength violated",
        json!({ "type": "string", "minLength": 3 }),
        json!("ab"),
        vec![MdvsKind::OutOfRange],
    );

    run(
        15, "maxLength violated",
        json!({ "type": "string", "maxLength": 5 }),
        json!("too long"),
        vec![MdvsKind::OutOfRange],
    );

    // ─── Pattern (TODO-0145) → WrongType ────────────────────────────────────
    run(
        16, "regex pattern mismatch",
        json!({ "type": "string", "pattern": "^[A-Z]+$" }),
        json!("lowercase"),
        vec![MdvsKind::WrongType],
    );

    // ─── Array bounds ───────────────────────────────────────────────────────
    run(
        17, "minItems violated",
        json!({ "type": "array", "minItems": 2 }),
        json!([1]),
        vec![MdvsKind::OutOfRange],
    );

    run(
        18, "maxItems violated",
        json!({ "type": "array", "maxItems": 2 }),
        json!([1, 2, 3]),
        vec![MdvsKind::OutOfRange],
    );

    run(
        19, "uniqueItems violated",
        json!({ "type": "array", "uniqueItems": true }),
        json!([1, 2, 2]),
        vec![MdvsKind::OutOfRange],
    );

    // ─── Combined: type error inside an array (item-level) ──────────────────
    run(
        20, "item-level type error inside array",
        json!({ "type": "array", "items": { "type": "string" } }),
        json!(["ok", 42, "also ok"]),
        vec![MdvsKind::WrongType],
    );

    // ─── Sanity: a nested-property type error resolves to the right kind ────
    run(
        21, "nested property wrong type",
        json!({
            "type": "object",
            "properties": {
                "draft": { "type": "boolean" }
            }
        }),
        json!({ "draft": "yes please" }),
        vec![MdvsKind::WrongType],
    );

    // ─── Two-violation merge: missing required + wrong type ─────────────────
    run(
        22, "missing required + wrong-type elsewhere",
        json!({
            "type": "object",
            "required": ["title"],
            "properties": {
                "title": { "type": "string" },
                "draft": { "type": "boolean" }
            }
        }),
        json!({ "draft": "yes please" }),
        vec![MdvsKind::MissingRequired, MdvsKind::WrongType],
    );

    println!("=== ValidationError → ViolationKind mapping verified ===");
}
