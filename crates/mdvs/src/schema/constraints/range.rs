//! Range constraint — config-time validation for `min`/`max` in TOML.
//!
//! Per-value validation is delegated to `jsonschema` via the `dsl_to_canonical`
//! translator in `schema/json_schema.rs`. This module only checks that the
//! constraint is well-formed at config load time.

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

/// Convert a TOML value to f64 for comparison.
fn toml_to_f64(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Integer(n) => Some(*n as f64),
        toml::Value::Float(f) => Some(*f),
        _ => None,
    }
}

/// Short human-readable name for a [`FieldType`] (for error messages).
fn field_type_name(ft: &FieldType) -> &'static str {
    match ft {
        FieldType::Boolean => "Boolean",
        FieldType::Integer => "Integer",
        FieldType::Float => "Float",
        FieldType::String => "String",
        FieldType::Date => "Date",
        FieldType::Array(_) => "Array",
        FieldType::Object(_) => "Object",
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

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
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
    fn type_date_rejects() {
        let err = validate_for_type("f", &FieldType::Date, &int_min(0), &int_max(1)).unwrap();
        assert!(err.contains("Date"));
        assert!(err.contains("does not apply"));
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
}
