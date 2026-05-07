//! Decode-direction integration tests.
//!
//! Lifted from `scripts/test_tomljson_decode.rs`. Each test parses a TOML
//! string and asserts the resulting JSON value (or expected error variant
//! for the non-representable-float cases).

use serde_json::{Value as Json, json};
use tomljson::{Error, TomlJsonOptions};

fn assert_decodes(toml_str: &str, expected: Json) {
    let actual = tomljson::from_str(toml_str).expect("decode succeeded");
    assert_eq!(actual, expected, "mismatch on input:\n{toml_str}");
}

fn assert_float_error(toml_str: &str, expected_kind: &'static str) {
    match tomljson::from_str(toml_str) {
        Err(Error::FloatNotRepresentable { kind, .. }) => {
            assert_eq!(
                kind, expected_kind,
                "wrong float kind for input:\n{toml_str}"
            );
        }
        Err(other) => panic!("expected FloatNotRepresentable, got {other:?}\ninput:\n{toml_str}"),
        Ok(j) => panic!("expected error, got {j}\ninput:\n{toml_str}"),
    }
}

// ============================================================================
// Standard scalars
// ============================================================================

#[test]
fn case_01_string() {
    assert_decodes(r#"x = "hello""#, json!({ "x": "hello" }));
}

#[test]
fn case_02_integer_positive() {
    assert_decodes("x = 42", json!({ "x": 42 }));
}

#[test]
fn case_03_integer_negative() {
    assert_decodes("x = -42", json!({ "x": -42 }));
}

#[test]
fn case_04_i64_max_boundary() {
    assert_decodes(
        "x = 9223372036854775807",
        json!({ "x": 9_223_372_036_854_775_807_i64 }),
    );
}

#[test]
fn case_05_i64_min_boundary() {
    assert_decodes(
        "x = -9223372036854775808",
        json!({ "x": -9_223_372_036_854_775_808_i64 }),
    );
}

#[test]
fn case_06_float_regular() {
    assert_decodes("x = 2.5", json!({ "x": 2.5 }));
}

#[test]
fn case_07_boolean_true() {
    assert_decodes("x = true", json!({ "x": true }));
}

#[test]
fn case_08_boolean_false() {
    assert_decodes("x = false", json!({ "x": false }));
}

// ============================================================================
// Datetime variants (all four → JSON string)
// ============================================================================

#[test]
fn case_09_local_date_to_string() {
    assert_decodes("x = 2026-05-04", json!({ "x": "2026-05-04" }));
}

#[test]
fn case_10_local_time_to_string() {
    assert_decodes("x = 09:30:00", json!({ "x": "09:30:00" }));
}

#[test]
fn case_11_local_datetime_to_string() {
    assert_decodes(
        "x = 2026-05-04T09:30:00",
        json!({ "x": "2026-05-04T09:30:00" }),
    );
}

#[test]
fn case_12_offset_datetime_z_to_string() {
    assert_decodes(
        "x = 2026-05-04T09:30:00Z",
        json!({ "x": "2026-05-04T09:30:00Z" }),
    );
}

#[test]
fn case_13_offset_datetime_with_offset_to_string() {
    assert_decodes(
        "x = 2026-05-04T09:30:00+02:00",
        json!({ "x": "2026-05-04T09:30:00+02:00" }),
    );
}

// ============================================================================
// Placeholder substitution
// ============================================================================

#[test]
fn case_14_placeholder_at_scalar_position() {
    assert_decodes(r#"default = "__null__""#, json!({ "default": null }));
}

#[test]
fn case_15_placeholder_inside_array() {
    assert_decodes(
        r#"enum = ["a", "b", "__null__"]"#,
        json!({ "enum": ["a", "b", null] }),
    );
}

#[test]
fn case_16_placeholder_inside_nested_object() {
    assert_decodes(
        "[default]\ncolor = \"__null__\"\n",
        json!({ "default": { "color": null } }),
    );
}

// ============================================================================
// Root unwrapping
// ============================================================================

#[test]
fn case_17_root_true() {
    assert_decodes("__root__ = true", json!(true));
}

#[test]
fn case_18_root_false() {
    assert_decodes("__root__ = false", json!(false));
}

#[test]
fn case_19_root_array() {
    assert_decodes("__root__ = [1, 2, 3]", json!([1, 2, 3]));
}

#[test]
fn case_20_root_scalar_string() {
    assert_decodes(r#"__root__ = "hello""#, json!("hello"));
}

// ============================================================================
// Recursion / nesting
// ============================================================================

#[test]
fn case_21_simple_key() {
    assert_decodes("x = 1", json!({ "x": 1 }));
}

#[test]
fn case_22_heterogeneous_array() {
    assert_decodes(
        r#"enum = [1, "two", true, "__null__"]"#,
        json!({ "enum": [1, "two", true, null] }),
    );
}

#[test]
fn case_23_nested_tables() {
    assert_decodes("[a.b]\nc = 1\n", json!({ "a": { "b": { "c": 1 } } }));
}

#[test]
fn case_24_inline_table() {
    assert_decodes(
        r#"x = { a = 1, b = "two" }"#,
        json!({ "x": { "a": 1, "b": "two" } }),
    );
}

#[test]
fn case_25_array_of_tables() {
    assert_decodes(
        "[[items]]\nname = \"first\"\n[[items]]\nname = \"second\"\n",
        json!({ "items": [{ "name": "first" }, { "name": "second" }] }),
    );
}

// ============================================================================
// Custom placeholder via TomlJsonOptions
// ============================================================================

#[test]
fn case_26_custom_placeholder_via_options() {
    let toml_str = r#"default = "@@NULL@@""#;
    let options = TomlJsonOptions {
        null_placeholder: "@@NULL@@".to_string(),
    };
    let actual = tomljson::from_str_with_options(toml_str, &options).expect("decode succeeded");
    assert_eq!(actual, json!({ "default": null }));
}

// ============================================================================
// Failure cases (NaN, +inf, -inf)
// ============================================================================

#[test]
fn case_27_plus_inf_errors() {
    assert_float_error("x = inf", "+inf");
}

#[test]
fn case_28_minus_inf_errors() {
    assert_float_error("x = -inf", "-inf");
}

#[test]
fn case_29_nan_errors() {
    assert_float_error("x = nan", "NaN");
}

#[test]
fn case_30_plus_inf_inside_array_errors() {
    assert_float_error("x = [1.0, inf, 2.0]", "+inf");
}

#[test]
fn case_31_nan_in_nested_table_errors() {
    assert_float_error("[constraints]\nupper = nan\n", "NaN");
}
