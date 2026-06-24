//! JSON-tree substitution for `<<NAME>>` placeholders in platform
//! templates. Lets per-platform `platform.toml` files declare arbitrary
//! envelope and config shapes without any Rust code knowing the details
//! of each harness's JSON layout.
//!
//! ## Marker syntax
//!
//! A marker is a JSON string whose **entire content** is `<<NAME>>` where
//! `NAME` is an uppercase identifier. The full-string-match rule means
//! `"hello <<NAME>>"` is NOT a marker; it's a literal string with text
//! that happens to contain angle brackets. This avoids the escaping and
//! partial-substitution edge cases of `printf`-style templating.
//!
//! ## Substitution rules
//!
//! Given a `vars: HashMap<&str, Option<String>>`:
//!
//! - `Some(value)` — the marker is replaced with that string value (as a
//!   JSON string node).
//! - `None` — the marker is **pruned**: its containing object key (or
//!   array element) is removed entirely. Use this for fields that don't
//!   apply to a particular invocation (e.g. `systemMessage` on search-
//!   nudge hooks that have no user-facing content).
//! - Marker name not present in `vars` — same as `None`, pruned.
//!
//! ## What this enables
//!
//! Per-platform JSON shapes — including substantially different ones —
//! live in `platform.toml` as data, not in Rust. Adding a new harness
//! with a novel envelope shape is one toml file.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// Walk `template` (a parsed JSON Value) and replace `<<NAME>>` placeholder
/// strings according to `vars`. See module docs for the substitution rules.
///
/// Returns a new `Value`; the input is left untouched.
pub fn substitute(template: &Value, vars: &HashMap<&str, Option<String>>) -> Value {
    match template {
        Value::String(s) => match parse_marker(s) {
            Some(name) => match vars.get(name) {
                Some(Some(value)) => Value::String(value.clone()),
                _ => Value::Null, // sentinel; only reachable if substitute is
                                  // called with a bare-marker root. Callers
                                  // walking objects/arrays prune before recursion.
            },
            None => Value::String(s.clone()),
        },
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                if should_prune(v, vars) {
                    continue;
                }
                out.insert(k.clone(), substitute(v, vars));
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let mut out = Vec::new();
            for item in items {
                if should_prune(item, vars) {
                    continue;
                }
                out.push(substitute(item, vars));
            }
            Value::Array(out)
        }
        other => other.clone(),
    }
}

/// If `s` is a bare marker (entire content matches `<<NAME>>`), return
/// `Some("NAME")`. Otherwise `None`.
fn parse_marker(s: &str) -> Option<&str> {
    s.strip_prefix("<<")
        .and_then(|s| s.strip_suffix(">>"))
        // Require non-empty name so `"<<>>"` isn't a marker.
        .filter(|n| !n.is_empty())
}

/// Should this value be pruned from its parent container? True only if
/// the value is a marker string whose name resolves to `None` (or isn't in
/// `vars` at all).
fn should_prune(value: &Value, vars: &HashMap<&str, Option<String>>) -> bool {
    let Value::String(s) = value else {
        return false;
    };
    let Some(name) = parse_marker(s) else {
        return false;
    };
    matches!(vars.get(name), Some(None) | None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars(pairs: &[(&'static str, Option<&str>)]) -> HashMap<&'static str, Option<String>> {
        pairs
            .iter()
            .map(|(k, v)| (*k, v.map(|s| s.to_string())))
            .collect()
    }

    #[test]
    fn marker_string_substitutes_when_value_present() {
        let template = json!({ "name": "<<NAME>>" });
        let out = substitute(&template, &vars(&[("NAME", Some("alice"))]));
        assert_eq!(out, json!({ "name": "alice" }));
    }

    #[test]
    fn marker_key_pruned_when_value_is_none() {
        let template = json!({
            "name": "<<NAME>>",
            "extra": "<<MISSING>>",
        });
        let out = substitute(
            &template,
            &vars(&[("NAME", Some("alice")), ("MISSING", None)]),
        );
        assert_eq!(out, json!({ "name": "alice" }));
    }

    #[test]
    fn marker_key_pruned_when_name_not_in_vars() {
        let template = json!({
            "name": "<<NAME>>",
            "extra": "<<UNDECLARED>>",
        });
        let out = substitute(&template, &vars(&[("NAME", Some("alice"))]));
        assert_eq!(out, json!({ "name": "alice" }));
    }

    #[test]
    fn partial_marker_left_as_literal() {
        // Only entire-string matches are substituted. Partial = literal.
        let template = json!({
            "greet": "hello <<NAME>>",
            "name": "<<NAME>>",
        });
        let out = substitute(&template, &vars(&[("NAME", Some("alice"))]));
        assert_eq!(out, json!({ "greet": "hello <<NAME>>", "name": "alice" }));
    }

    #[test]
    fn nested_objects_substituted() {
        let template = json!({
            "outer": {
                "inner": "<<X>>",
                "kept": "<<Y>>",
            }
        });
        let out = substitute(&template, &vars(&[("X", Some("foo")), ("Y", Some("bar"))]));
        assert_eq!(out, json!({ "outer": { "inner": "foo", "kept": "bar" } }));
    }

    #[test]
    fn nested_object_prunes_only_keys_not_outer_value() {
        // Inner marker missing → inner key pruned. Outer remains as {}.
        let template = json!({
            "outer": { "inner": "<<MISSING>>" }
        });
        let out = substitute(&template, &vars(&[("MISSING", None)]));
        assert_eq!(out, json!({ "outer": {} }));
    }

    #[test]
    fn array_elements_pruned() {
        let template = json!(["<<A>>", "<<MISSING>>", "<<B>>", "literal",]);
        let out = substitute(
            &template,
            &vars(&[("A", Some("a")), ("B", Some("b")), ("MISSING", None)]),
        );
        assert_eq!(out, json!(["a", "b", "literal"]));
    }

    #[test]
    fn non_string_values_pass_through() {
        let template = json!({
            "n": 42,
            "b": true,
            "x": null,
            "a": [1, 2, 3],
            "s": "<<NAME>>",
        });
        let out = substitute(&template, &vars(&[("NAME", Some("alice"))]));
        assert_eq!(
            out,
            json!({ "n": 42, "b": true, "x": null, "a": [1, 2, 3], "s": "alice" })
        );
    }

    #[test]
    fn empty_marker_treated_as_literal() {
        // `<<>>` is NOT a valid marker (no name), so it should pass through.
        let template = json!({ "field": "<<>>" });
        let out = substitute(&template, &HashMap::new());
        assert_eq!(out, json!({ "field": "<<>>" }));
    }

    #[test]
    fn empty_string_substitutes_to_empty_string() {
        // An empty-string value is Some(""), not None. The key stays in
        // the output with an empty string. Important: this differs from
        // pruning. Callers can use Some("") to mean "include with empty
        // body" or None to mean "remove the field entirely".
        let template = json!({ "msg": "<<MSG>>" });
        let out = substitute(&template, &vars(&[("MSG", Some(""))]));
        assert_eq!(out, json!({ "msg": "" }));
    }

    #[test]
    fn claude_code_validate_envelope_full_substitution() {
        // Realistic test: a Claude-Code-shaped envelope template, both
        // markers populated.
        let template = json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": "<<MSG>>"
            },
            "systemMessage": "<<USER_MSG>>"
        });
        let out = substitute(
            &template,
            &vars(&[
                ("MSG", Some("agent body")),
                ("USER_MSG", Some("pretty body")),
            ]),
        );
        assert_eq!(
            out,
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "agent body"
                },
                "systemMessage": "pretty body"
            })
        );
    }

    #[test]
    fn claude_code_search_nudge_envelope_prunes_user_msg() {
        // Realistic test: search-nudge has no user message, so the
        // systemMessage key is pruned.
        let template = json!({
            "hookSpecificOutput": {
                "hookEventName": "PostToolUse",
                "additionalContext": "<<MSG>>"
            },
            "systemMessage": "<<USER_MSG>>"
        });
        let out = substitute(
            &template,
            &vars(&[("MSG", Some("tip body")), ("USER_MSG", None)]),
        );
        assert_eq!(
            out,
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": "tip body"
                }
            })
        );
    }

    #[test]
    fn cursor_envelope_only_uses_msg() {
        // Cursor's shape: snake_case field, no wrapper, no user channel
        // from postToolUse. The USER_MSG var isn't referenced anywhere in
        // the template — whether it's Some or None makes no difference.
        let template = json!({ "additional_context": "<<MSG>>" });
        let with_user = substitute(
            &template,
            &vars(&[("MSG", Some("body")), ("USER_MSG", Some("ignored"))]),
        );
        let without_user = substitute(
            &template,
            &vars(&[("MSG", Some("body")), ("USER_MSG", None)]),
        );
        assert_eq!(with_user, json!({ "additional_context": "body" }));
        assert_eq!(without_user, json!({ "additional_context": "body" }));
    }

    #[test]
    fn realistic_config_template_with_multiple_markers() {
        // Mirrors what scaffold::hook will substitute.
        let template = json!({
            "hooks": {
                "PostToolUse": [
                    {
                        "matcher": "Edit|Write|MultiEdit",
                        "hooks": [
                            { "type": "command", "command": "<<COMMAND_VALIDATE>>" }
                        ]
                    },
                    {
                        "matcher": "Bash",
                        "hooks": [
                            { "type": "command", "command": "<<COMMAND_SEARCH>>" }
                        ]
                    }
                ]
            }
        });
        let out = substitute(
            &template,
            &vars(&[
                (
                    "COMMAND_VALIDATE",
                    Some("mdvs hook handle --platform claude-code --kind validate"),
                ),
                (
                    "COMMAND_SEARCH",
                    Some("mdvs hook handle --platform claude-code --kind search-nudge"),
                ),
            ]),
        );
        let validate = &out["hooks"]["PostToolUse"][0]["hooks"][0]["command"];
        assert_eq!(
            validate.as_str().unwrap(),
            "mdvs hook handle --platform claude-code --kind validate"
        );
    }
}
