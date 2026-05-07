//! Range constraint — validation logic for `min`/`max` in TOML.

use super::ConstraintViolation;
use crate::discover::field_type::FieldType;

/// Check that `min`/`max` are applicable to `field_type` and well-formed.
///
/// Rules:
/// - Only numeric types: Integer, Float, Array(Integer), Array(Float)
/// - Integer fields require integer bounds (float bounds rejected)
/// - Float fields accept both integer and float bounds (widened to f64)
/// - If both min and max present, min must be <= max
pub(super) fn validate_for_type(
    field_name: &str,
    field_type: &FieldType,
    min: &Option<toml::Value>,
    max: &Option<toml::Value>,
) -> Option<String> {
    let element_type = match field_type {
        FieldType::Integer => FieldType::Integer,
        FieldType::Float => FieldType::Float,
        FieldType::Array(inner) => match inner.as_ref() {
            FieldType::Integer => FieldType::Integer,
            FieldType::Float => FieldType::Float,
            other => {
                return Some(format!(
                    "field '{field_name}': range constraint does not apply \
                     to Array({}) fields — only Array(Integer) and \
                     Array(Float) are supported",
                    field_type_name(other),
                ));
            }
        },
        other => {
            return Some(format!(
                "field '{field_name}': range constraint does not apply \
                 to {} fields — only Integer, Float, Array(Integer), \
                 and Array(Float) are supported",
                field_type_name(other),
            ));
        }
    };

    // Validate bound types match the element type.
    if let Some(v) = min
        && let Some(err) = validate_bound_type(field_name, "min", v, &element_type)
    {
        return Some(err);
    }
    if let Some(v) = max
        && let Some(err) = validate_bound_type(field_name, "max", v, &element_type)
    {
        return Some(err);
    }

    // If both present, check min <= max.
    if let (Some(min_v), Some(max_v)) = (min, max) {
        let min_f = toml_to_f64(min_v);
        let max_f = toml_to_f64(max_v);
        if let (Some(lo), Some(hi)) = (min_f, max_f)
            && lo > hi
        {
            return Some(format!(
                "field '{field_name}': min ({}) is greater than max ({})",
                format_toml_num(min_v),
                format_toml_num(max_v),
            ));
        }
    }

    None
}

/// Check a single frontmatter value against a range constraint.
pub(super) fn validate_value(
    value: &serde_json::Value,
    field_type: &FieldType,
    min: &Option<toml::Value>,
    max: &Option<toml::Value>,
) -> Option<ConstraintViolation> {
    let rule = format_range_rule(min, max);

    match field_type {
        FieldType::Integer | FieldType::Float => {
            if value.is_null() {
                return None;
            }
            check_scalar(value, min, max, &rule)
        }
        FieldType::Array(_) => {
            if let serde_json::Value::Array(arr) = value {
                let bad: Vec<String> = arr
                    .iter()
                    .filter(|elem| check_scalar(elem, min, max, &rule).is_some())
                    .map(format_json_num)
                    .collect();
                if bad.is_empty() {
                    None
                } else {
                    Some(ConstraintViolation {
                        rule,
                        detail: format!("elements out of range: [{}]", bad.join(", ")),
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

/// Validate that a single bound value is numeric and matches the element type.
fn validate_bound_type(
    field_name: &str,
    bound_name: &str,
    bound: &toml::Value,
    element_type: &FieldType,
) -> Option<String> {
    match (element_type, bound) {
        // Integer field: only integer bounds allowed.
        (FieldType::Integer, toml::Value::Integer(_)) => None,
        (FieldType::Integer, toml::Value::Float(_)) => Some(format!(
            "field '{field_name}': {bound_name} is a float but field type is Integer \
             — use an integer bound",
        )),
        // Float field: integer or float bounds (widened to f64).
        (FieldType::Float, toml::Value::Integer(_) | toml::Value::Float(_)) => None,
        // Non-numeric bound value.
        _ => Some(format!(
            "field '{field_name}': {bound_name} must be a numeric value, got {}",
            bound.type_str(),
        )),
    }
}

/// Check a scalar JSON value against min/max bounds.
fn check_scalar(
    value: &serde_json::Value,
    min: &Option<toml::Value>,
    max: &Option<toml::Value>,
    rule: &str,
) -> Option<ConstraintViolation> {
    let n = json_to_f64(value)?;

    if let Some(min_v) = min
        && let Some(lo) = toml_to_f64(min_v)
        && n < lo
    {
        return Some(ConstraintViolation {
            rule: rule.to_string(),
            detail: format!("got {}", format_json_num(value)),
        });
    }

    if let Some(max_v) = max
        && let Some(hi) = toml_to_f64(max_v)
        && n > hi
    {
        return Some(ConstraintViolation {
            rule: rule.to_string(),
            detail: format!("got {}", format_json_num(value)),
        });
    }

    None
}

/// Convert a TOML value to f64 for comparison.
fn toml_to_f64(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Integer(n) => Some(*n as f64),
        toml::Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// Convert a JSON number to f64 for comparison.
fn json_to_f64(v: &serde_json::Value) -> Option<f64> {
    v.as_f64()
}

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

/// Format the range rule string for violation messages.
fn format_range_rule(min: &Option<toml::Value>, max: &Option<toml::Value>) -> String {
    match (min, max) {
        (Some(lo), Some(hi)) => {
            format!(
                "min = {}, max = {}",
                format_toml_num(lo),
                format_toml_num(hi)
            )
        }
        (Some(lo), None) => format!("min = {}", format_toml_num(lo)),
        (None, Some(hi)) => format!("max = {}", format_toml_num(hi)),
        (None, None) => String::new(),
    }
}

/// Format a TOML numeric value for display.
fn format_toml_num(v: &toml::Value) -> String {
    match v {
        toml::Value::Integer(n) => n.to_string(),
        toml::Value::Float(f) => format!("{f}"),
        _ => v.to_string(),
    }
}

/// Format a JSON numeric value for display.
fn format_json_num(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(f) = n.as_f64() {
                format!("{f}")
            } else {
                n.to_string()
            }
        }
        other => other.to_string(),
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    // -- helpers --

    fn int_min(n: i64) -> Option<toml::Value> {
        Some(toml::Value::Integer(n))
    }

    fn int_max(n: i64) -> Option<toml::Value> {
        Some(toml::Value::Integer(n))
    }

    fn float_min(f: f64) -> Option<toml::Value> {
        Some(toml::Value::Float(f))
    }

    fn float_max(f: f64) -> Option<toml::Value> {
        Some(toml::Value::Float(f))
    }

    // -----------------------------------------------------------------------
    // validate_for_type — type applicability
    // -----------------------------------------------------------------------

    #[test]
    fn type_integer_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, &int_min(0), &int_max(10)).is_none());
    }

    #[test]
    fn type_float_accepts() {
        assert!(
            validate_for_type("f", &FieldType::Float, &float_min(0.0), &float_max(1.0)).is_none()
        );
    }

    #[test]
    fn type_float_accepts_integer_bounds() {
        assert!(validate_for_type("f", &FieldType::Float, &int_min(0), &int_max(100)).is_none());
    }

    #[test]
    fn type_array_integer_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(validate_for_type("f", &ft, &int_min(1), &int_max(10)).is_none());
    }

    #[test]
    fn type_array_float_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        assert!(validate_for_type("f", &ft, &float_min(0.0), &float_max(1.0)).is_none());
    }

    #[test]
    fn type_boolean_rejects() {
        let err = validate_for_type("f", &FieldType::Boolean, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("Boolean"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn type_string_rejects() {
        let err = validate_for_type("f", &FieldType::String, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("String"));
    }

    #[test]
    fn type_object_rejects() {
        let ft = FieldType::Object(BTreeMap::new());
        let err = validate_for_type("f", &ft, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("Object"));
    }

    #[test]
    fn type_array_string_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        let err = validate_for_type("f", &ft, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("Array(String)"));
    }

    #[test]
    fn type_array_boolean_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::Boolean));
        let err = validate_for_type("f", &ft, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("Array(Boolean)"));
    }

    // -----------------------------------------------------------------------
    // validate_for_type — bound type validation
    // -----------------------------------------------------------------------

    #[test]
    fn integer_field_float_bound_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, &float_min(0.5), &None).unwrap();
        assert!(err.contains("float"));
        assert!(err.contains("Integer"));
    }

    #[test]
    fn integer_field_float_max_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, &None, &float_max(10.5)).unwrap();
        assert!(err.contains("float"));
    }

    #[test]
    fn float_field_mixed_bounds_accepts() {
        // Integer min, float max on a float field — widening.
        assert!(validate_for_type("f", &FieldType::Float, &int_min(0), &float_max(1.0)).is_none());
    }

    #[test]
    fn string_bound_rejects() {
        let bad = Some(toml::Value::String("hello".into()));
        let err = validate_for_type("f", &FieldType::Integer, &bad, &None).unwrap();
        assert!(err.contains("numeric"));
    }

    // -----------------------------------------------------------------------
    // validate_for_type — min > max
    // -----------------------------------------------------------------------

    #[test]
    fn min_greater_than_max_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, &int_min(10), &int_max(5)).unwrap();
        assert!(err.contains("greater than"));
    }

    #[test]
    fn min_equals_max_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, &int_min(5), &int_max(5)).is_none());
    }

    #[test]
    fn min_only_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, &int_min(0), &None).is_none());
    }

    #[test]
    fn max_only_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, &None, &int_max(100)).is_none());
    }

    // -----------------------------------------------------------------------
    // validate_value — integer scalars
    // -----------------------------------------------------------------------

    #[test]
    fn value_integer_in_range() {
        assert!(
            validate_value(&json!(5), &FieldType::Integer, &int_min(1), &int_max(10)).is_none()
        );
    }

    #[test]
    fn value_integer_below_min() {
        let v = validate_value(&json!(0), &FieldType::Integer, &int_min(1), &int_max(10)).unwrap();
        assert!(v.detail.contains("0"));
        assert!(v.rule.contains("min = 1"));
    }

    #[test]
    fn value_integer_above_max() {
        let v = validate_value(&json!(15), &FieldType::Integer, &int_min(1), &int_max(10)).unwrap();
        assert!(v.detail.contains("15"));
        assert!(v.rule.contains("max = 10"));
    }

    #[test]
    fn value_integer_at_min_boundary() {
        assert!(
            validate_value(&json!(1), &FieldType::Integer, &int_min(1), &int_max(10)).is_none()
        );
    }

    #[test]
    fn value_integer_at_max_boundary() {
        assert!(
            validate_value(&json!(10), &FieldType::Integer, &int_min(1), &int_max(10)).is_none()
        );
    }

    #[test]
    fn value_integer_negative() {
        assert!(
            validate_value(&json!(-5), &FieldType::Integer, &int_min(-10), &int_max(-1)).is_none()
        );
    }

    // -----------------------------------------------------------------------
    // validate_value — float scalars
    // -----------------------------------------------------------------------

    #[test]
    fn value_float_in_range() {
        assert!(
            validate_value(
                &json!(0.5),
                &FieldType::Float,
                &float_min(0.0),
                &float_max(1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn value_float_below_min() {
        let v = validate_value(
            &json!(-0.1),
            &FieldType::Float,
            &float_min(0.0),
            &float_max(1.0),
        )
        .unwrap();
        assert!(v.detail.contains("-0.1"));
    }

    #[test]
    fn value_float_above_max() {
        let v = validate_value(
            &json!(1.5),
            &FieldType::Float,
            &float_min(0.0),
            &float_max(1.0),
        )
        .unwrap();
        assert!(v.detail.contains("1.5"));
    }

    #[test]
    fn value_float_with_integer_bounds() {
        // Float value compared against integer bounds (widened).
        assert!(
            validate_value(&json!(50.5), &FieldType::Float, &int_min(0), &int_max(100)).is_none()
        );
    }

    // -----------------------------------------------------------------------
    // validate_value — min-only / max-only
    // -----------------------------------------------------------------------

    #[test]
    fn value_min_only_passes() {
        assert!(validate_value(&json!(100), &FieldType::Integer, &int_min(0), &None).is_none());
    }

    #[test]
    fn value_min_only_fails() {
        let v = validate_value(&json!(-1), &FieldType::Integer, &int_min(0), &None).unwrap();
        assert!(v.rule.contains("min = 0"));
        assert!(!v.rule.contains("max"));
    }

    #[test]
    fn value_max_only_passes() {
        assert!(validate_value(&json!(5), &FieldType::Integer, &None, &int_max(10)).is_none());
    }

    #[test]
    fn value_max_only_fails() {
        let v = validate_value(&json!(15), &FieldType::Integer, &None, &int_max(10)).unwrap();
        assert!(v.rule.contains("max = 10"));
        assert!(!v.rule.contains("min"));
    }

    // -----------------------------------------------------------------------
    // validate_value — null and type mismatch
    // -----------------------------------------------------------------------

    #[test]
    fn value_null_passthrough() {
        assert!(
            validate_value(&json!(null), &FieldType::Integer, &int_min(0), &int_max(10)).is_none()
        );
    }

    #[test]
    fn value_string_passthrough() {
        // Non-numeric value on numeric field — type checker handles this, not range.
        assert!(
            validate_value(
                &json!("hello"),
                &FieldType::Integer,
                &int_min(0),
                &int_max(10)
            )
            .is_none()
        );
    }

    // -----------------------------------------------------------------------
    // validate_value — arrays
    // -----------------------------------------------------------------------

    #[test]
    fn value_array_integer_all_in_range() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(validate_value(&json!([1, 5, 10]), &ft, &int_min(1), &int_max(10)).is_none());
    }

    #[test]
    fn value_array_integer_some_out_of_range() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        let v = validate_value(&json!([1, 15, -1]), &ft, &int_min(0), &int_max(10)).unwrap();
        assert!(v.detail.contains("15"));
        assert!(v.detail.contains("-1"));
        assert!(v.detail.contains("elements out of range"));
    }

    #[test]
    fn value_array_float_all_in_range() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        assert!(
            validate_value(
                &json!([0.1, 0.5, 0.9]),
                &ft,
                &float_min(0.0),
                &float_max(1.0),
            )
            .is_none()
        );
    }

    #[test]
    fn value_array_float_some_out_of_range() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        let v = validate_value(
            &json!([0.5, 1.5, -0.1]),
            &ft,
            &float_min(0.0),
            &float_max(1.0),
        )
        .unwrap();
        assert!(v.detail.contains("1.5"));
        assert!(v.detail.contains("-0.1"));
    }

    #[test]
    fn value_array_empty_passes() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(validate_value(&json!([]), &ft, &int_min(0), &int_max(10)).is_none());
    }

    #[test]
    fn value_non_array_on_array_field_passthrough() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(validate_value(&json!(5), &ft, &int_min(0), &int_max(10)).is_none());
    }

    // -----------------------------------------------------------------------
    // format helpers
    // -----------------------------------------------------------------------

    #[test]
    fn format_rule_both_bounds() {
        let rule = format_range_rule(&int_min(1), &int_max(10));
        assert_eq!(rule, "min = 1, max = 10");
    }

    #[test]
    fn format_rule_min_only() {
        let rule = format_range_rule(&int_min(0), &None);
        assert_eq!(rule, "min = 0");
    }

    #[test]
    fn format_rule_max_only() {
        let rule = format_range_rule(&None, &int_max(100));
        assert_eq!(rule, "max = 100");
    }

    #[test]
    fn format_rule_float_bounds() {
        let rule = format_range_rule(&float_min(0.0), &float_max(1.0));
        assert_eq!(rule, "min = 0, max = 1");
    }
}
