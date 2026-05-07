#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde_json = "1"
//! jsonschema = "0.46"
//! globset = "0.4"
//! ```
//!
//! Spike for path-scoped validation (TODO-0149, "Path-scoped validation" section).
//!
//! Validates the design that mdvs's path-aware `allowed` / `required` globs
//! are NOT encoded in JSON Schema directly. Instead:
//!
//!   1. The translator partitions the schema once at config-load time:
//!      strip `x-mdvs.allowed` / `x-mdvs.required` from every property,
//!      hold them aside as a `PathScopeMap`. The stripped schema is
//!      compiled by `jsonschema::Validator`.
//!
//!   2. Per file, mdvs synthesizes a tiny "overlay" schema encoding
//!      which fields are permitted / required for that filepath, and
//!      validates the frontmatter against it (in addition to the
//!      stripped global schema).
//!
//! The script exercises:
//!   - partition: x-mdvs extraction + default-scope handling
//!   - overlay synthesis: per-file `properties` / `required` / `additionalProperties`
//!   - double-validation: type errors caught by global, presence errors by overlay
//!   - `[fields].ignore` interaction (permissive properties)
//!   - unknown fields rejected by overlay (mapped to NewField in real mdvs)
//!
//! Run: `rust-script scripts/test_path_scoped_validation.rs`

use globset::{Glob, GlobSet, GlobSetBuilder};
use jsonschema::Validator;
use serde_json::{json, Value as Json};
use std::collections::BTreeMap;

// ============================================================================
// Path scope map: per-property allowed / required globs
// ============================================================================

struct PathScope {
    allowed: GlobSet,
    required: GlobSet,
}

type PathScopeMap = BTreeMap<String, PathScope>;

fn build_globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).expect("valid glob"));
    }
    b.build().expect("valid globset")
}

// ============================================================================
// Step 1: partition the schema (strip x-mdvs.allowed / x-mdvs.required)
// ============================================================================

fn partition_schema(mut schema: Json) -> (Json, PathScopeMap) {
    let mut scope_map = PathScopeMap::new();
    partition_node(&mut schema, &mut scope_map, None);
    (schema, scope_map)
}

fn partition_node(node: &mut Json, scope_map: &mut PathScopeMap, prop_name: Option<&str>) {
    let Json::Object(obj) = node else { return };

    if let Some(name) = prop_name {
        // Default: allowed everywhere, never required.
        let mut allowed_pats: Vec<String> = vec!["**".into()];
        let mut required_pats: Vec<String> = vec![];

        if let Some(Json::Object(xmdvs)) = obj.get_mut("x-mdvs") {
            if let Some(v) = xmdvs.remove("allowed") {
                allowed_pats = serde_json::from_value(v).expect("allowed: array of glob strings");
            }
            if let Some(v) = xmdvs.remove("required") {
                required_pats =
                    serde_json::from_value(v).expect("required: array of glob strings");
            }
            // If x-mdvs is now empty after stripping our keys, remove the key entirely
            // so the stripped schema is mdvs-agnostic.
            let now_empty = xmdvs.is_empty();
            if now_empty {
                obj.remove("x-mdvs");
            }
        }

        scope_map.insert(
            name.to_string(),
            PathScope {
                allowed: build_globset(&allowed_pats),
                required: build_globset(&required_pats),
            },
        );
    }

    if let Some(Json::Object(props)) = obj.get_mut("properties") {
        for (name, prop) in props.iter_mut() {
            // Clone the name out — we can't borrow props while mutating its values.
            let name_owned = name.clone();
            partition_node(prop, scope_map, Some(&name_owned));
        }
    }
}

// ============================================================================
// Step 2: per-file overlay schema synthesis
// ============================================================================

fn synthesize_overlay(filepath: &str, scope_map: &PathScopeMap, ignored: &[&str]) -> Json {
    let mut props = serde_json::Map::new();
    let mut required: Vec<String> = vec![];

    for (name, scope) in scope_map {
        if scope.allowed.is_match(filepath) {
            // `{}` accepts any value — type/constraint checking is the global schema's job.
            props.insert(name.clone(), json!({}));
            if scope.required.is_match(filepath) {
                required.push(name.clone());
            }
        }
    }

    for f in ignored {
        props.insert((*f).to_string(), json!({}));
    }

    json!({
        "type": "object",
        "properties": props,
        "required": required,
        "additionalProperties": false,
    })
}

// ============================================================================
// Step 3: double-validation
// ============================================================================

#[derive(Debug, PartialEq, Eq, Hash)]
enum Violation {
    /// jsonschema rejected on type/constraint (global schema)
    TypeOrConstraint { path: String, keyword: String },
    /// overlay rejected — field present but forbidden for this path
    DisallowedField(String),
    /// overlay rejected — field required for this path but absent
    MissingRequired(String),
}

fn validate(
    filepath: &str,
    instance: &Json,
    global: &Validator,
    overlay: &Validator,
) -> Vec<Violation> {
    let mut out = vec![];

    for err in global.iter_errors(instance) {
        let path = err.instance_path().to_string();
        let keyword = format!("{:?}", err.kind()).split_whitespace().next().unwrap_or("?").to_string();
        out.push(Violation::TypeOrConstraint { path, keyword });
    }

    for err in overlay.iter_errors(instance) {
        let kind_str = format!("{:?}", err.kind());
        if kind_str.contains("AdditionalProperties") {
            let path = err.instance_path().to_string();
            let name = path.trim_start_matches('/').to_string();
            if name.is_empty() {
                for n in extract_unexpected_names(&kind_str) {
                    out.push(Violation::DisallowedField(n));
                }
            } else {
                out.push(Violation::DisallowedField(name));
            }
        } else if kind_str.contains("Required") {
            for n in extract_required_names(&kind_str) {
                out.push(Violation::MissingRequired(n));
            }
        } else {
            out.push(Violation::TypeOrConstraint {
                path: format!("overlay:{}", err.instance_path()),
                keyword: kind_str,
            });
        }
    }

    let _ = filepath; // unused but kept in signature for clarity
    out
}

/// Pull property names from a Debug-formatted `AdditionalProperties` error kind.
fn extract_unexpected_names(s: &str) -> Vec<String> {
    // jsonschema 0.46's Debug for AdditionalProperties looks like:
    //   AdditionalProperties { unexpected: ["draft"] }
    extract_quoted_names_after(s, "unexpected")
}

fn extract_required_names(s: &str) -> Vec<String> {
    // Required { property: Value::String("draft") } or similar shapes
    extract_quoted_names_after(s, "")
}

fn extract_quoted_names_after(s: &str, _marker: &str) -> Vec<String> {
    let mut out = vec![];
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j] != b'"' {
                j += 1;
            }
            if j < bytes.len() {
                out.push(std::str::from_utf8(&bytes[start..j]).unwrap().to_string());
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

// ============================================================================
// Test driver
// ============================================================================

fn run(num: usize, name: &str, filepath: &str, instance: Json, expected: Vec<Violation>,
       global: &Validator, scope_map: &PathScopeMap, ignored: &[&str]) {
    println!("--- {}. {} ---", num, name);
    println!("file:     {}", filepath);
    println!("instance: {}", instance);

    let overlay_schema = synthesize_overlay(filepath, scope_map, ignored);
    let overlay = jsonschema::validator_for(&overlay_schema)
        .expect("overlay schema must compile");

    let actual = validate(filepath, &instance, global, &overlay);
    println!("actual:   {:?}", actual);
    println!("expected: {:?}", expected);

    // Compare as sets (order doesn't matter)
    let actual_set: std::collections::HashSet<_> = actual.into_iter().collect();
    let expected_set: std::collections::HashSet<_> = expected.into_iter().collect();
    assert_eq!(actual_set, expected_set, "mismatch on test {} ({})", num, name);
    println!("  {}. {}  ✓\n", num, name);
}

fn main() {
    println!("=== Path-scoped validation spike ===\n");

    // Canonical schema with three fields, exercising different glob shapes.
    let raw_schema = json!({
        "type": "object",
        "properties": {
            "title": {
                "type": "string",
                "x-mdvs": {
                    "allowed": ["**"],
                    "required": ["**"]
                }
            },
            "draft": {
                "type": "boolean",
                "x-mdvs": {
                    "allowed": ["docs/**"],
                    "required": ["docs/published/**"]
                }
            },
            "tags": {
                "type": "array",
                "items": { "type": "string" }
                // No x-mdvs — defaults: allowed everywhere, never required.
            }
        }
    });

    let (stripped, scope_map) = partition_schema(raw_schema);

    println!("stripped schema:\n{}\n", serde_json::to_string_pretty(&stripped).unwrap());
    println!("scope_map keys: {:?}\n", scope_map.keys().collect::<Vec<_>>());

    // Sanity: stripped schema should compile.
    let global = jsonschema::validator_for(&stripped).expect("global schema must compile");

    // Sanity: stripped schema should have no `x-mdvs` keys anywhere.
    let stripped_str = serde_json::to_string(&stripped).unwrap();
    assert!(!stripped_str.contains("x-mdvs"), "x-mdvs not fully stripped");
    println!("(verified: x-mdvs fully stripped from compiled schema)\n");

    // Ignored fields list (mdvs `[fields].ignore`).
    let ignored: &[&str] = &["author", "date"];

    // ─── Permitted + required + present + correct ───────────────────────────
    run(1, "docs/published file with all fields correct",
        "docs/published/launch.md",
        json!({ "title": "Launch", "draft": true, "tags": ["release"] }),
        vec![],
        &global, &scope_map, ignored);

    // ─── Permitted + required + absent ──────────────────────────────────────
    run(2, "docs/published file missing required `draft`",
        "docs/published/missing.md",
        json!({ "title": "Hi" }),
        vec![Violation::MissingRequired("draft".into())],
        &global, &scope_map, ignored);

    // ─── Forbidden + present ────────────────────────────────────────────────
    run(3, "notes/ file with disallowed `draft`",
        "notes/random.md",
        json!({ "title": "Hi", "draft": true }),
        vec![Violation::DisallowedField("draft".into())],
        &global, &scope_map, ignored);

    // ─── Permitted + optional + absent ──────────────────────────────────────
    run(4, "docs/ file (not published) with no `draft`",
        "docs/drafts/wip.md",
        json!({ "title": "WIP" }),
        vec![],  // draft permitted but not required here
        &global, &scope_map, ignored);

    // ─── Permitted + optional + present ─────────────────────────────────────
    run(5, "docs/ file (not published) with `draft` set",
        "docs/drafts/wip.md",
        json!({ "title": "WIP", "draft": false }),
        vec![],
        &global, &scope_map, ignored);

    // ─── Type error (global validator) ──────────────────────────────────────
    run(6, "docs/published file with wrong type for `draft`",
        "docs/published/bad.md",
        json!({ "title": "Bad", "draft": "yes please" }),
        vec![Violation::TypeOrConstraint {
            path: "/draft".into(),
            keyword: "Type".into(),
        }],
        &global, &scope_map, ignored);

    // ─── Combined: type error AND presence error ────────────────────────────
    run(7, "notes/ file with disallowed + wrong-type `draft`",
        "notes/oops.md",
        json!({ "title": "Oops", "draft": 42 }),
        vec![
            Violation::TypeOrConstraint { path: "/draft".into(), keyword: "Type".into() },
            Violation::DisallowedField("draft".into()),
        ],
        &global, &scope_map, ignored);

    // ─── Ignored field present → pass ───────────────────────────────────────
    run(8, "ignored fields (author, date) accepted anywhere",
        "notes/anywhere.md",
        json!({ "title": "Hi", "author": "edo", "date": "2026-05-06" }),
        vec![],
        &global, &scope_map, ignored);

    // ─── Truly unknown field → DisallowedField (NewField in real mdvs) ──────
    run(9, "unknown field `secret` rejected as additional",
        "notes/secret.md",
        json!({ "title": "Hi", "secret": "xyz" }),
        vec![Violation::DisallowedField("secret".into())],
        &global, &scope_map, ignored);

    // ─── Multiple violations of different kinds ─────────────────────────────
    run(10, "missing title + disallowed draft + unknown key",
        "notes/triple.md",
        json!({ "draft": true, "rogue": 1 }),
        vec![
            Violation::MissingRequired("title".into()),
            Violation::DisallowedField("draft".into()),
            Violation::DisallowedField("rogue".into()),
        ],
        &global, &scope_map, ignored);

    // ─── Glob `**` matches deeply nested paths ──────────────────────────────
    run(11, "deeply nested path under docs/published/",
        "docs/published/sub/dir/launch.md",
        json!({ "title": "Deep", "draft": true }),
        vec![],
        &global, &scope_map, ignored);

    // ─── Tags array (no x-mdvs scoping → default allowed everywhere) ────────
    run(12, "tags optional everywhere",
        "anywhere/file.md",
        json!({ "title": "Tagged", "tags": ["a", "b"] }),
        vec![],
        &global, &scope_map, ignored);

    println!("=== All path-scoped validation cases pass ===");
}
