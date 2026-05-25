#!/usr/bin/env rust-script
//! Spike for TODO-0162 step 1.
//!
//! Question: how does `gray_matter`'s TOML engine represent native TOML
//! `Date` and `DateTime` values when it returns a `Pod`? TOML lets you
//! write `joined = 2024-03-14` and `synced_at = 2024-03-14T10:25:00Z`
//! unquoted — those are real typed values in the language, not strings.
//! Our downstream pipeline assumes JSON-compatible types and expects
//! dates as strings.
//!
//! Outcomes:
//!   A — Pod returns dates as strings in "YYYY-MM-DD" / RFC 3339 form:
//!       no design change needed. Existing Date / DateTime inference
//!       picks them up clean.
//!   B — Pod returns them as a structured / tagged / typed variant:
//!       add one conversion arm in the Pod→JSON layer that emits the
//!       string form before downstream inference sees it.
//!
//! Compare with YAML parsing of the same logical content as a sanity
//! check so we can see the per-engine difference at a glance.
//!
//! Note: gray_matter's TOML and JSON engines are feature-gated (default
//! is yaml-only). The real Cargo.toml change in TODO-0162 step 2 will
//! need `gray_matter = { version = "0.3", features = ["toml", "json"] }`.
//!
//! ```cargo
//! [dependencies]
//! gray_matter = { version = "0.3", features = ["toml", "json"] }
//! serde_json = "1"
//! ```

use gray_matter::engine::{TOML, YAML};
use gray_matter::{Matter, Pod};

const TOML_INPUT: &str = r#"+++
title = "A note with TOML frontmatter"
joined = 2024-03-14
synced_at = 2024-03-14T10:25:00Z
explicit_string_date = "2024-03-14"
count = 42
draft = false
tags = ["alpha", "beta"]
+++

# Body

Some body text.
"#;

const YAML_INPUT: &str = r#"---
title: A note with YAML frontmatter
joined: 2024-03-14
synced_at: 2024-03-14T10:25:00Z
explicit_string_date: "2024-03-14"
count: 42
draft: false
tags:
  - alpha
  - beta
---

# Body

Some body text.
"#;

fn kind_label(v: &serde_json::Value) -> &'static str {
    match v {
        serde_json::Value::String(_) => "string",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::Object(_) => "object (structured!)",
        serde_json::Value::Array(_) => "array (structured!)",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Null => "null",
    }
}

fn dump_engine<E: gray_matter::engine::Engine>(label: &str, input: &str, delimiter: &str) {
    println!("=== {} engine (delimiter = {:?}) ===", label, delimiter);
    let mut matter: Matter<E> = Matter::new();
    matter.delimiter = delimiter.to_string();
    let parsed = match matter.parse::<Pod>(input) {
        Ok(p) => p,
        Err(e) => {
            println!("parse error: {}", e);
            return;
        }
    };
    let pod = match parsed.data {
        Some(p) => p,
        None => {
            println!("No frontmatter parsed.");
            return;
        }
    };
    println!("Pod (Debug):\n  {:#?}", pod);

    let as_json: serde_json::Value = match pod.deserialize() {
        Ok(v) => v,
        Err(e) => {
            println!("Pod→serde_json::Value error: {}", e);
            return;
        }
    };
    println!(
        "As serde_json::Value:\n{}",
        serde_json::to_string_pretty(&as_json).unwrap()
    );
    if let Some(joined) = as_json.get("joined") {
        println!("  joined               = {} ({})", joined, kind_label(joined));
    }
    if let Some(synced) = as_json.get("synced_at") {
        println!("  synced_at            = {} ({})", synced, kind_label(synced));
    }
    if let Some(esd) = as_json.get("explicit_string_date") {
        println!("  explicit_string_date = {} ({})", esd, kind_label(esd));
    }
    println!();
}

fn main() {
    // CRITICAL: `Matter::new()` defaults to delimiter = "---" regardless
    // of the engine. We MUST set `matter.delimiter = "+++"` for TOML.
    // This is something the real implementation in TODO-0162 will need
    // to do per-engine when dispatching.
    dump_engine::<TOML>("TOML", TOML_INPUT, "+++");
    dump_engine::<YAML>("YAML", YAML_INPUT, "---");

    println!("=== Key question ===");
    println!("Look at TOML's `joined` and `synced_at`:");
    println!("  if both are 'string' → Outcome A: no design change.");
    println!("  if either is 'object'/'number' → Outcome B: add one Pod→JSON arm.");
}
