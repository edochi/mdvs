//! Pattern constraint — config-time validation for `pattern` (regex).
//!
//! Per-value validation is delegated to `jsonschema` via the `dsl_to_canonical`
//! translator. This module only checks the constraint is applicable and that
//! the regex compiles at config load time.

use crate::discover::field_type::FieldType;

/// Check that `pattern` is applicable to `field_type` and that the regex is
/// well-formed.
///
/// Rules:
/// - Only String and Array(String) fields.
/// - The pattern string must compile as a regex (`regex::Regex::new`).
pub(super) fn validate_for_type(
    field_name: &str,
    field_type: &FieldType,
    pattern: &str,
) -> Option<String> {
    let applicable = match field_type {
        FieldType::String => true,
        FieldType::Array(inner) => matches!(**inner, FieldType::String),
        _ => false,
    };
    if !applicable {
        return Some(format!(
            "field '{field_name}': pattern constraint does not apply \
             to {} fields — only String and Array(String) are supported",
            field_type_name(field_type),
        ));
    }

    if let Err(e) = regex::Regex::new(pattern) {
        return Some(format!(
            "field '{field_name}': pattern is not a valid regex: {e}"
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
    fn validate_for_type_string_accepts_valid_regex() {
        assert!(validate_for_type("f", &FieldType::String, "^[A-Z]+$").is_none());
    }

    #[test]
    fn validate_for_type_array_string_accepts() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        assert!(validate_for_type("f", &ft, r"\d+").is_none());
    }

    #[test]
    fn validate_for_type_invalid_regex_rejects() {
        let err = validate_for_type("f", &FieldType::String, "[unclosed").unwrap();
        assert!(err.contains("not a valid regex"));
    }

    #[test]
    fn validate_for_type_integer_rejects() {
        let err = validate_for_type("f", &FieldType::Integer, "^[A-Z]+$").unwrap();
        assert!(err.contains("Integer"));
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn validate_for_type_float_rejects() {
        let err = validate_for_type("f", &FieldType::Float, "^[A-Z]+$").unwrap();
        assert!(err.contains("Float"));
    }

    #[test]
    fn validate_for_type_boolean_rejects() {
        let err = validate_for_type("f", &FieldType::Boolean, "^[A-Z]+$").unwrap();
        assert!(err.contains("Boolean"));
    }

    #[test]
    fn validate_for_type_array_integer_rejects() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        let err = validate_for_type("f", &ft, "^[A-Z]+$").unwrap();
        assert!(err.contains("does not apply"));
    }

    #[test]
    fn validate_for_type_empty_pattern_compiles() {
        // Empty regex matches empty string — valid regex, even if rarely useful.
        assert!(validate_for_type("f", &FieldType::String, "").is_none());
    }
}
