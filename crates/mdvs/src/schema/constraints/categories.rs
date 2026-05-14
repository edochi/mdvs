//! Categories constraint — config-time validation for `categories = [...]` in TOML.
//!
//! Per-value validation is delegated to `jsonschema` via the `dsl_to_canonical`
//! translator in `schema/json_schema.rs`. This module only checks that the
//! constraint is well-formed at config load time.

use crate::discover::field_type::FieldType;

/// Check that `categories` is applicable to `field_type` and that all
/// category values match the expected element type.
pub(super) fn validate_for_type(
    field_name: &str,
    field_type: &FieldType,
    values: &[toml::Value],
) -> Option<String> {
    // Date/DateTime categorical values are TOML strings on disk (e.g.
    // "2024-01-15"); the runtime jsonschema `format: date` /
    // `format: date-time` validators catch invalid ones.
    let element_type = match field_type {
        FieldType::String | FieldType::Date | FieldType::DateTime => FieldType::String,
        FieldType::Integer => FieldType::Integer,
        FieldType::Array(inner) => match inner.as_ref() {
            FieldType::String | FieldType::Date | FieldType::DateTime => FieldType::String,
            FieldType::Integer => FieldType::Integer,
            other => {
                return Some(format!(
                    "field '{field_name}': categories constraint does not apply \
                     to Array({}) fields — only Array(String), Array(Integer), \
                     Array(Date), and Array(DateTime) are supported",
                    field_type_name(other),
                ));
            }
        },
        other => {
            return Some(format!(
                "field '{field_name}': categories constraint does not apply \
                 to {} fields — only String, Integer, Date, DateTime, Array(String), \
                 Array(Integer), Array(Date), and Array(DateTime) are supported",
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
        // The element_type narrowing above only produces String or Integer.
        // Any other variant here is a code-path bug — treat the value as
        // failing rather than panic.
        let ok = match &element_type {
            FieldType::String => val.is_str(),
            FieldType::Integer => val.is_integer(),
            _ => false,
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
        FieldType::Date => "Date",
        FieldType::DateTime => "DateTime",
        FieldType::Array(_) => "Array",
        FieldType::Object(_) => "Object",
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::constraints::ConstraintKind;
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
    fn validate_for_type_date_accepts_string_categories() {
        // Date categorical values are TOML strings on disk; jsonschema's
        // format:date validator catches invalid ones at runtime.
        let k = cats_kind(str_cats(&["2024-01-01", "2024-12-31"]));
        assert!(k.validate_for_type("f", &FieldType::Date).is_none());
    }

    #[test]
    fn validate_for_type_array_date_accepts_string_categories() {
        let k = cats_kind(str_cats(&["2024-01-01", "2024-12-31"]));
        let ft = FieldType::Array(Box::new(FieldType::Date));
        assert!(k.validate_for_type("f", &ft).is_none());
    }

    #[test]
    fn validate_for_type_datetime_accepts_string_categories() {
        let k = cats_kind(str_cats(&["2024-01-01T00:00:00Z", "2024-12-31T23:59:59Z"]));
        assert!(k.validate_for_type("f", &FieldType::DateTime).is_none());
    }

    #[test]
    fn validate_for_type_array_datetime_accepts_string_categories() {
        let k = cats_kind(str_cats(&["2024-01-01T00:00:00Z", "2024-12-31T23:59:59Z"]));
        let ft = FieldType::Array(Box::new(FieldType::DateTime));
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
}
