//! Categories constraint — validation logic for `categories = [...]` in TOML.

use super::ConstraintViolation;
use crate::discover::field_type::FieldType;

/// Check that `categories` is applicable to `field_type` and that all
/// category values match the expected element type.
pub(super) fn validate_for_type(
    field_name: &str,
    field_type: &FieldType,
    values: &[toml::Value],
) -> Option<String> {
    let element_type = match field_type {
        FieldType::String => FieldType::String,
        FieldType::Integer => FieldType::Integer,
        FieldType::Array(inner) => match inner.as_ref() {
            FieldType::String => FieldType::String,
            FieldType::Integer => FieldType::Integer,
            other => {
                return Some(format!(
                    "field '{field_name}': categories constraint does not apply \
                     to Array({}) fields — only Array(String) and \
                     Array(Integer) are supported",
                    field_type_name(other),
                ));
            }
        },
        other => {
            return Some(format!(
                "field '{field_name}': categories constraint does not apply \
                 to {} fields — only String, Integer, Array(String), \
                 and Array(Integer) are supported",
                field_type_name(other),
            ));
        }
    };

    if values.is_empty() {
        return Some(format!(
            "field '{field_name}': categories list must not be empty"
        ));
    }

    for (i, val) in values.iter().enumerate() {
        let ok = match &element_type {
            FieldType::String => val.is_str(),
            FieldType::Integer => val.is_integer(),
            _ => unreachable!(),
        };
        if !ok {
            return Some(format!(
                "field '{field_name}': category value at index {i} ({val}) \
                 does not match field type {}",
                field_type_name(&element_type),
            ));
        }
    }

    None
}

/// Check a single frontmatter value against a categories constraint.
pub(super) fn validate_value(
    value: &serde_json::Value,
    field_type: &FieldType,
    categories: &[toml::Value],
) -> Option<ConstraintViolation> {
    let rule = format_categories_rule(categories);

    match field_type {
        FieldType::String | FieldType::Integer => {
            if value.is_null() {
                return None;
            }
            if !json_value_in_toml_categories(value, categories) {
                Some(ConstraintViolation {
                    rule,
                    detail: format!("got {}", format_json_value(value)),
                })
            } else {
                None
            }
        }
        FieldType::Array(_) => {
            if let serde_json::Value::Array(arr) = value {
                let bad: Vec<String> = arr
                    .iter()
                    .filter(|elem| !json_value_in_toml_categories(elem, categories))
                    .map(format_json_value)
                    .collect();
                if bad.is_empty() {
                    None
                } else {
                    Some(ConstraintViolation {
                        rule,
                        detail: format!("got {}", bad.join(", ")),
                    })
                }
            } else {
                // Type mismatch — handled by the type checker, not constraints.
                None
            }
        }
        // Other types: constraint doesn't apply (caught at config time).
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Short human-readable name for a [`FieldType`] (for error messages).
fn field_type_name(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::Boolean => "Boolean",
        FieldType::Integer => "Integer",
        FieldType::Float => "Float",
        FieldType::String => "String",
        FieldType::Array(_) => "Array",
        FieldType::Object(_) => "Object",
    }
}

/// Check whether a single JSON value matches any of the TOML category values.
fn json_value_in_toml_categories(value: &serde_json::Value, categories: &[toml::Value]) -> bool {
    categories.iter().any(|cat| toml_json_eq(cat, value))
}

/// Compare a TOML category value with a JSON frontmatter value for equality.
fn toml_json_eq(toml_val: &toml::Value, json_val: &serde_json::Value) -> bool {
    match (toml_val, json_val) {
        (toml::Value::String(t), serde_json::Value::String(j)) => t == j,
        (toml::Value::Integer(t), serde_json::Value::Number(j)) => {
            j.as_i64().is_some_and(|n| n == *t)
        }
        _ => false,
    }
}

/// Format the categories list as a rule string for violation messages.
fn format_categories_rule(categories: &[toml::Value]) -> String {
    let items: Vec<String> = categories.iter().map(|v| v.to_string()).collect();
    format!("categories = [{}]", items.join(", "))
}

/// Format a JSON value for violation detail messages.
fn format_json_value(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{s}\""),
        serde_json::Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::constraints::ConstraintKind;
    use serde_json::json;
    use std::collections::BTreeMap;

    // -- helpers --

    fn str_cats(vals: &[&str]) -> Vec<toml::Value> {
        vals.iter()
            .map(|s| toml::Value::String(s.to_string()))
            .collect()
    }

    fn int_cats(vals: &[i64]) -> Vec<toml::Value> {
        vals.iter().map(|&n| toml::Value::Integer(n)).collect()
    }

    fn cats_kind(cats: Vec<toml::Value>) -> ConstraintKind {
        ConstraintKind::Categories(cats)
    }

    // -----------------------------------------------------------------------
    // validate_for_type — type applicability
    // -----------------------------------------------------------------------

    #[test]
    fn validate_for_type_string_accepts() {
        let k = cats_kind(str_cats(&["a", "b"]));
        assert!(k.validate_for_type("f", &FieldType::String).is_none());
    }

    #[test]
    fn validate_for_type_integer_accepts() {
        let k = cats_kind(int_cats(&[1, 2]));
        assert!(k.validate_for_type("f", &FieldType::Integer).is_none());
    }

    #[test]
    fn validate_for_type_array_string_accepts() {
        let k = cats_kind(str_cats(&["a", "b"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(k.validate_for_type("f", &ft).is_none());
    }

    #[test]
    fn validate_for_type_array_integer_accepts() {
        let k = cats_kind(int_cats(&[1, 2]));
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(k.validate_for_type("f", &ft).is_none());
    }

    #[test]
    fn validate_for_type_boolean_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let err = k.validate_for_type("f", &FieldType::Boolean).unwrap();
        assert!(err.contains("Boolean"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn validate_for_type_float_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let err = k.validate_for_type("f", &FieldType::Float).unwrap();
        assert!(err.contains("Float"));
    }

    #[test]
    fn validate_for_type_object_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let ft = FieldType::Object(BTreeMap::new());
        let err = k.validate_for_type("f", &ft).unwrap();
        assert!(err.contains("Object"));
    }

    #[test]
    fn validate_for_type_array_float_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let ft = FieldType::Array(Box::new(FieldType::Float));
        let err = k.validate_for_type("f", &ft).unwrap();
        assert!(err.contains("Array(Float)"));
    }

    #[test]
    fn validate_for_type_array_boolean_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let ft = FieldType::Array(Box::new(FieldType::Boolean));
        let err = k.validate_for_type("f", &ft).unwrap();
        assert!(err.contains("Array(Boolean)"));
    }

    #[test]
    fn validate_for_type_array_object_rejects() {
        let k = cats_kind(str_cats(&["a"]));
        let ft = FieldType::Array(Box::new(FieldType::Object(BTreeMap::new())));
        let err = k.validate_for_type("f", &ft).unwrap();
        assert!(err.contains("Array(Object)"));
    }

    // -----------------------------------------------------------------------
    // validate_for_type — category value validation
    // -----------------------------------------------------------------------

    #[test]
    fn validate_for_type_string_field_integer_values_rejects() {
        let k = cats_kind(int_cats(&[1, 2]));
        let err = k.validate_for_type("f", &FieldType::String).unwrap();
        assert!(err.contains("index 0"));
        assert!(err.contains("String"));
    }

    #[test]
    fn validate_for_type_integer_field_string_values_rejects() {
        let k = cats_kind(str_cats(&["a", "b"]));
        let err = k.validate_for_type("f", &FieldType::Integer).unwrap();
        assert!(err.contains("index 0"));
        assert!(err.contains("Integer"));
    }

    #[test]
    fn validate_for_type_mixed_category_values_rejects() {
        let cats = vec![
            toml::Value::String("draft".into()),
            toml::Value::Integer(42),
        ];
        let k = cats_kind(cats);
        let err = k.validate_for_type("f", &FieldType::String).unwrap();
        assert!(err.contains("index 1"));
    }

    #[test]
    fn validate_for_type_empty_categories_rejects() {
        let k = cats_kind(vec![]);
        let err = k.validate_for_type("f", &FieldType::String).unwrap();
        assert!(err.contains("must not be empty"));
    }

    // -----------------------------------------------------------------------
    // validate_value — string categories
    // -----------------------------------------------------------------------

    #[test]
    fn validate_value_string_match() {
        let k = cats_kind(str_cats(&["draft", "published"]));
        assert!(
            k.validate_value(&json!("draft"), &FieldType::String)
                .is_none()
        );
    }

    #[test]
    fn validate_value_string_no_match() {
        let k = cats_kind(str_cats(&["draft", "published"]));
        let v = k
            .validate_value(&json!("pending"), &FieldType::String)
            .unwrap();
        assert!(v.detail.contains("\"pending\""));
        assert!(v.rule.contains("draft"));
    }

    #[test]
    fn validate_value_string_case_sensitive() {
        let k = cats_kind(str_cats(&["draft", "published"]));
        assert!(
            k.validate_value(&json!("Draft"), &FieldType::String)
                .is_some()
        );
    }

    // -----------------------------------------------------------------------
    // validate_value — integer categories
    // -----------------------------------------------------------------------

    #[test]
    fn validate_value_integer_match() {
        let k = cats_kind(int_cats(&[1, 2, 3]));
        assert!(k.validate_value(&json!(2), &FieldType::Integer).is_none());
    }

    #[test]
    fn validate_value_integer_no_match() {
        let k = cats_kind(int_cats(&[1, 2, 3]));
        let v = k.validate_value(&json!(5), &FieldType::Integer).unwrap();
        assert!(v.detail.contains("5"));
    }

    #[test]
    fn validate_value_negative_integer() {
        let k = cats_kind(int_cats(&[-1, 0, 1]));
        assert!(k.validate_value(&json!(-1), &FieldType::Integer).is_none());
    }

    // -----------------------------------------------------------------------
    // validate_value — array categories
    // -----------------------------------------------------------------------

    #[test]
    fn validate_value_array_string_all_match() {
        let k = cats_kind(str_cats(&["rust", "python", "go"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(k.validate_value(&json!(["rust", "go"]), &ft).is_none());
    }

    #[test]
    fn validate_value_array_string_some_no_match() {
        let k = cats_kind(str_cats(&["rust", "python", "go"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        let v = k.validate_value(&json!(["rust", "java"]), &ft).unwrap();
        assert!(v.detail.contains("\"java\""));
        assert!(!v.detail.contains("\"rust\""));
    }

    #[test]
    fn validate_value_array_string_multiple_no_match() {
        let k = cats_kind(str_cats(&["rust", "python", "go"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        let v = k.validate_value(&json!(["java", "c++"]), &ft).unwrap();
        assert!(v.detail.contains("\"java\""));
        assert!(v.detail.contains("\"c++\""));
    }

    #[test]
    fn validate_value_array_integer_all_match() {
        let k = cats_kind(int_cats(&[1, 2, 3]));
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(k.validate_value(&json!([1, 3]), &ft).is_none());
    }

    #[test]
    fn validate_value_array_integer_some_no_match() {
        let k = cats_kind(int_cats(&[1, 2, 3]));
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        let v = k.validate_value(&json!([1, 5]), &ft).unwrap();
        assert!(v.detail.contains("5"));
    }

    #[test]
    fn validate_value_empty_array_passes() {
        let k = cats_kind(str_cats(&["rust", "python"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(k.validate_value(&json!([]), &ft).is_none());
    }

    // -----------------------------------------------------------------------
    // validate_value — edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn validate_value_null_passthrough() {
        let k = cats_kind(str_cats(&["draft", "published"]));
        assert!(k.validate_value(&json!(null), &FieldType::String).is_none());
    }

    #[test]
    fn validate_value_non_array_on_array_field_passthrough() {
        let k = cats_kind(str_cats(&["rust", "python"]));
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(k.validate_value(&json!("rust"), &ft).is_none());
    }

    // -----------------------------------------------------------------------
    // toml_json_eq helper
    // -----------------------------------------------------------------------

    #[test]
    fn toml_json_eq_string_match() {
        assert!(toml_json_eq(
            &toml::Value::String("hello".into()),
            &json!("hello"),
        ));
    }

    #[test]
    fn toml_json_eq_string_no_match() {
        assert!(!toml_json_eq(
            &toml::Value::String("hello".into()),
            &json!("world"),
        ));
    }

    #[test]
    fn toml_json_eq_integer_match() {
        assert!(toml_json_eq(&toml::Value::Integer(42), &json!(42)));
    }

    #[test]
    fn toml_json_eq_integer_no_match() {
        assert!(!toml_json_eq(&toml::Value::Integer(42), &json!(99)));
    }

    #[test]
    fn toml_json_eq_cross_type_no_match() {
        assert!(!toml_json_eq(&toml::Value::String("42".into()), &json!(42),));
        assert!(!toml_json_eq(&toml::Value::Integer(42), &json!("42"),));
    }

    // -----------------------------------------------------------------------
    // format helpers
    // -----------------------------------------------------------------------

    #[test]
    fn format_rule_strings() {
        let cats = str_cats(&["draft", "published"]);
        let rule = format_categories_rule(&cats);
        assert_eq!(rule, r#"categories = ["draft", "published"]"#);
    }

    #[test]
    fn format_rule_integers() {
        let cats = int_cats(&[1, 2, 3]);
        let rule = format_categories_rule(&cats);
        assert_eq!(rule, "categories = [1, 2, 3]");
    }
}
