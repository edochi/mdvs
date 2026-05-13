//! Length constraint — config-time validation for `min_length`/`max_length`.
//!
//! Per-value validation is delegated to `jsonschema` via the `dsl_to_canonical`
//! translator. This module only checks the constraint is applicable and
//! well-formed at config load time.

use crate::discover::field_type::FieldType;

/// Check that `min_length`/`max_length` are applicable to `field_type` and
/// well-formed.
///
/// Rules:
/// - Only String and Array(String) fields.
/// - At least one of `min` / `max` must be set.
/// - If both present, `min` must be ≤ `max`.
pub(super) fn validate_for_type(
    field_name: &str,
    field_type: &FieldType,
    min: Option<u64>,
    max: Option<u64>,
) -> Option<String> {
    // Type applicability.
    let applicable = match field_type {
        FieldType::String => true,
        FieldType::Array(inner) => matches!(**inner, FieldType::String),
        _ => false,
    };
    if !applicable {
        return Some(format!(
            "field '{field_name}': length constraint does not apply \
             to {} fields — only String and Array(String) are supported",
            field_type_name(field_type),
        ));
    }

    // At least one bound must be set.
    if min.is_none() && max.is_none() {
        return Some(format!(
            "field '{field_name}': length constraint must have min_length or max_length set"
        ));
    }

    // min ≤ max if both present.
    if let (Some(lo), Some(hi)) = (min, max)
        && lo > hi
    {
        return Some(format!(
            "field '{field_name}': min_length ({lo}) is greater than max_length ({hi})"
        ));
    }

    None
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_for_type_string_accepts() {
        assert!(validate_for_type("f", &FieldType::String, Some(1), Some(64)).is_none());
    }

    #[test]
    fn validate_for_type_array_string_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(validate_for_type("f", &ft, Some(3), Some(10)).is_none());
    }

    #[test]
    fn validate_for_type_only_min_accepts() {
        assert!(validate_for_type("f", &FieldType::String, Some(3), None).is_none());
    }

    #[test]
    fn validate_for_type_only_max_accepts() {
        assert!(validate_for_type("f", &FieldType::String, None, Some(64)).is_none());
    }

    #[test]
    fn validate_for_type_integer_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, Some(0), Some(10)).unwrap();
        assert!(err.contains("Integer"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn validate_for_type_float_rejects() {
        let err = validate_for_type("f", &FieldType::Float, Some(0), Some(10)).unwrap();
        assert!(err.contains("Float"));
    }

    #[test]
    fn validate_for_type_boolean_rejects() {
        let err = validate_for_type("f", &FieldType::Boolean, Some(0), Some(10)).unwrap();
        assert!(err.contains("Boolean"));
    }

    #[test]
    fn validate_for_type_array_integer_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        let err = validate_for_type("f", &ft, Some(0), Some(10)).unwrap();
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn validate_for_type_neither_bound_rejects() {
        let err = validate_for_type("f", &FieldType::String, None, None).unwrap();
        assert!(err.contains("must have"));
    }

    #[test]
    fn validate_for_type_min_greater_than_max_rejects() {
        let err = validate_for_type("f", &FieldType::String, Some(10), Some(5)).unwrap();
        assert!(err.contains("greater than"));
    }

    #[test]
    fn validate_for_type_min_equal_max_accepts() {
        assert!(validate_for_type("f", &FieldType::String, Some(5), Some(5)).is_none());
    }
}
