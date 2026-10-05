//! Range constraint — config-time validation for `min`/`max` in TOML.
//!
//! Per-value validation is delegated to `jsonschema` via the `dsl_to_canonical`
//! translator in `schema/json_schema.rs`. This module only checks that the
//! constraint is well-formed at config load time.

use crate::{discover::field_type::FieldType, num::i64_to_f64_exact};

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
    min: Option<&toml::Value>,
    max: Option<&toml::Value>,
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
    if let (Some(min_v), Some(max_v)) = (min, max)
        && min_exceeds_max(min_v, max_v)
    {
        return Some(format!(
            "field '{field_name}': min ({}) is greater than max ({})",
            format_toml_num(min_v),
            format_toml_num(max_v),
        ));
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
        // Float field: integer or float bounds; integers widen to f64 and must
        // therefore convert exactly.
        (FieldType::Integer, toml::Value::Integer(_))
        | (FieldType::Float, toml::Value::Float(_)) => None,
        (FieldType::Float, toml::Value::Integer(n)) => match i64_to_f64_exact(*n) {
            Some(_) => None,
            None => Some(format!(
                "field '{field_name}': {bound_name} ({n}) is beyond ±2^53 and has no exact \
                 Float equivalent — use a smaller integer or a float bound",
            )),
        },
        (FieldType::Integer, toml::Value::Float(_)) => Some(format!(
            "field '{field_name}': {bound_name} is a float but field type is Integer \
             — use an integer bound",
        )),
        // Non-numeric bound value.
        _ => Some(format!(
            "field '{field_name}': {bound_name} must be a numeric value, got {}",
            bound.type_str(),
        )),
    }
}

/// Whether `min_v` is greater than `max_v`.
///
/// Two integer bounds compare exactly in i64; any other pair compares in f64.
/// Integer bounds on Float fields were already checked to convert exactly, so
/// a bound that does not convert only arises for non-numeric values, which
/// are reported elsewhere.
fn min_exceeds_max(min_v: &toml::Value, max_v: &toml::Value) -> bool {
    if let (toml::Value::Integer(lo), toml::Value::Integer(hi)) = (min_v, max_v) {
        return lo > hi;
    }
    matches!(
        (bound_to_f64(min_v), bound_to_f64(max_v)),
        (Some(lo), Some(hi)) if lo > hi
    )
}

/// Convert a numeric TOML bound to f64; `None` for non-numeric values and for
/// integers beyond ±2^53, which f64 cannot hold exactly.
fn bound_to_f64(v: &toml::Value) -> Option<f64> {
    match v {
        toml::Value::Integer(n) => i64_to_f64_exact(*n),
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
        FieldType::DateTime => "DateTime",
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
    use std::collections::BTreeMap;

    use super::*;
    use crate::num::F64_EXACT_INT_LIMIT_I64;

    // -- helpers --

    fn int_min(n: i64) -> toml::Value {
        toml::Value::Integer(n)
    }

    fn int_max(n: i64) -> toml::Value {
        toml::Value::Integer(n)
    }

    fn float_min(f: f64) -> toml::Value {
        toml::Value::Float(f)
    }

    fn float_max(f: f64) -> toml::Value {
        toml::Value::Float(f)
    }

    // -----------------------------------------------------------------------
    // validate_for_type — type applicability
    // -----------------------------------------------------------------------

    #[test]
    fn type_integer_accepts() {
        assert!(
            validate_for_type(
                "f",
                &FieldType::Integer,
                Some(&int_min(0)),
                Some(&int_max(10))
            )
            .is_none()
        );
    }

    #[test]
    fn type_float_accepts() {
        assert!(
            validate_for_type(
                "f",
                &FieldType::Float,
                Some(&float_min(0.0)),
                Some(&float_max(1.0))
            )
            .is_none()
        );
    }

    #[test]
    fn type_float_accepts_integer_bounds() {
        assert!(
            validate_for_type(
                "f",
                &FieldType::Float,
                Some(&int_min(0)),
                Some(&int_max(100))
            )
            .is_none()
        );
    }

    #[test]
    fn type_array_integer_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        assert!(validate_for_type("f", &ft, Some(&int_min(1)), Some(&int_max(10))).is_none());
    }

    #[test]
    fn type_array_float_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        assert!(
            validate_for_type("f", &ft, Some(&float_min(0.0)), Some(&float_max(1.0))).is_none()
        );
    }

    #[test]
    fn type_boolean_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::Boolean,
            Some(&int_min(0)),
            Some(&int_max(1)),
        )
        .unwrap();
        assert!(err.contains("Boolean"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn type_string_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::String,
            Some(&int_min(0)),
            Some(&int_max(1)),
        )
        .unwrap();
        assert!(err.contains("String"));
    }

    #[test]
    fn type_date_rejects() {
        let err =
            validate_for_type("f", &FieldType::Date, Some(&int_min(0)), Some(&int_max(1))).unwrap();
        assert!(err.contains("Date"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn type_datetime_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::DateTime,
            Some(&int_min(0)),
            Some(&int_max(1)),
        )
        .unwrap();
        assert!(err.contains("DateTime"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn type_object_rejects() {
        let ft = FieldType::Object(BTreeMap::new());
        let err = validate_for_type("f", &ft, Some(&int_min(0)), Some(&int_max(1))).unwrap();
        assert!(err.contains("Object"));
    }

    #[test]
    fn type_array_string_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        let err = validate_for_type("f", &ft, Some(&int_min(0)), Some(&int_max(1))).unwrap();
        assert!(err.contains("Array(String)"));
    }

    #[test]
    fn type_array_boolean_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::Boolean));
        let err = validate_for_type("f", &ft, Some(&int_min(0)), Some(&int_max(1))).unwrap();
        assert!(err.contains("Array(Boolean)"));
    }

    // -----------------------------------------------------------------------
    // validate_for_type — bound type validation
    // -----------------------------------------------------------------------

    #[test]
    fn integer_field_float_bound_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, Some(&float_min(0.5)), None).unwrap();
        assert!(err.contains("float"));
        assert!(err.contains("Integer"));
    }

    #[test]
    fn integer_field_float_max_rejects() {
        let err =
            validate_for_type("f", &FieldType::Integer, None, Some(&float_max(10.5))).unwrap();
        assert!(err.contains("float"));
    }

    #[test]
    fn float_field_mixed_bounds_accepts() {
        // Integer min, float max on a float field — widening.
        assert!(
            validate_for_type(
                "f",
                &FieldType::Float,
                Some(&int_min(0)),
                Some(&float_max(1.0))
            )
            .is_none()
        );
    }

    #[test]
    fn string_bound_rejects() {
        let bad = toml::Value::String("hello".into());
        let err = validate_for_type("f", &FieldType::Integer, Some(&bad), None).unwrap();
        assert!(err.contains("numeric"));
    }

    // -----------------------------------------------------------------------
    // validate_for_type — min > max
    // -----------------------------------------------------------------------

    #[test]
    fn min_greater_than_max_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::Integer,
            Some(&int_min(10)),
            Some(&int_max(5)),
        )
        .unwrap();
        assert!(err.contains("greater than"));
    }

    #[test]
    fn integer_field_bounds_beyond_f64_precision_compare_exactly() {
        // As f64 both bounds round to 2^53 and would look equal.
        let err = validate_for_type(
            "f",
            &FieldType::Integer,
            Some(&int_min(F64_EXACT_INT_LIMIT_I64 + 1)),
            Some(&int_max(F64_EXACT_INT_LIMIT_I64)),
        )
        .unwrap();
        assert!(err.contains("greater than"));
    }

    #[test]
    fn integer_field_accepts_bounds_beyond_f64_precision() {
        assert!(
            validate_for_type(
                "f",
                &FieldType::Integer,
                Some(&int_min(-F64_EXACT_INT_LIMIT_I64 - 1)),
                Some(&int_max(F64_EXACT_INT_LIMIT_I64 + 1)),
            )
            .is_none()
        );
    }

    #[test]
    fn float_field_lone_integer_min_beyond_f64_precision_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::Float,
            Some(&int_min(F64_EXACT_INT_LIMIT_I64 + 1)),
            None,
        )
        .unwrap();
        assert!(err.contains("beyond"));
    }

    #[test]
    fn float_field_integer_pair_beyond_f64_precision_rejects() {
        let err = validate_for_type(
            "f",
            &FieldType::Float,
            Some(&int_min(0)),
            Some(&int_max(F64_EXACT_INT_LIMIT_I64 + 1)),
        )
        .unwrap();
        assert!(err.contains("beyond"));
    }

    #[test]
    fn array_float_field_mixed_bounds_beyond_f64_precision_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        let err = validate_for_type(
            "f",
            &ft,
            Some(&int_min(-F64_EXACT_INT_LIMIT_I64 - 1)),
            Some(&float_max(1.0)),
        )
        .unwrap();
        assert!(err.contains("beyond"));
    }

    #[test]
    fn float_field_integer_bound_at_f64_precision_limit_compares() {
        let err = validate_for_type(
            "f",
            &FieldType::Float,
            Some(&int_min(F64_EXACT_INT_LIMIT_I64)),
            Some(&float_max(1.0)),
        )
        .unwrap();
        assert!(err.contains("greater than"));
    }

    #[test]
    fn min_equals_max_accepts() {
        assert!(
            validate_for_type(
                "f",
                &FieldType::Integer,
                Some(&int_min(5)),
                Some(&int_max(5))
            )
            .is_none()
        );
    }

    #[test]
    fn min_only_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, Some(&int_min(0)), None).is_none());
    }

    #[test]
    fn max_only_accepts() {
        assert!(validate_for_type("f", &FieldType::Integer, None, Some(&int_max(100))).is_none());
    }
}
