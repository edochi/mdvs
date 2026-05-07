//! Range constraint inference — compute min/max from observed numeric values.

use crate::discover::field_type::FieldType;
use crate::discover::infer::InferredField;

/// Infer min and max bounds from observed numeric values.
///
/// Returns `Some((min_toml, max_toml))` if the field type is numeric and
/// at least one numeric value was observed. Returns `None` for non-numeric
/// types or when no values are available.
///
/// Integer fields produce `toml::Value::Integer` bounds.
/// Float fields produce `toml::Value::Float` bounds.
pub fn infer(field: &InferredField) -> Option<(toml::Value, toml::Value)> {
    let is_float = match &field.field_type {
        FieldType::Integer => false,
        FieldType::Float => true,
        FieldType::Array(inner) => match inner.as_ref() {
            FieldType::Integer => false,
            FieldType::Float => true,
            _ => return None,
        },
        _ => return None,
    };

    let nums: Vec<f64> = field
        .distinct_values
        .iter()
        .filter_map(|v| v.as_f64())
        .collect();

    if nums.is_empty() {
        return None;
    }

    let min = nums.iter().copied().fold(f64::INFINITY, f64::min);
    let max = nums.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    if is_float {
        Some((toml::Value::Float(min), toml::Value::Float(max)))
    } else {
        Some((
            toml::Value::Integer(min as i64),
            toml::Value::Integer(max as i64),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_field(name: &str, ft: FieldType, distinct: Vec<serde_json::Value>) -> InferredField {
        let count = distinct.len();
        InferredField {
            name: name.into(),
            field_type: ft,
            files: vec![],
            allowed: vec![],
            required: vec![],
            nullable: false,
            distinct_values: distinct,
            occurrence_count: count,
        }
    }

    #[test]
    fn infer_integer_range() {
        let f = make_field(
            "count",
            FieldType::Integer,
            vec![json!(5), json!(10), json!(3)],
        );
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Integer(3));
        assert_eq!(max, toml::Value::Integer(10));
    }

    #[test]
    fn infer_float_range() {
        let f = make_field(
            "temp",
            FieldType::Float,
            vec![json!(0.5), json!(2.3), json!(1.1)],
        );
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Float(0.5));
        assert_eq!(max, toml::Value::Float(2.3));
    }

    #[test]
    fn infer_array_integer_range() {
        let ft = FieldType::Array(Box::new(FieldType::Integer));
        let f = make_field("ratings", ft, vec![json!(1), json!(5), json!(3)]);
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Integer(1));
        assert_eq!(max, toml::Value::Integer(5));
    }

    #[test]
    fn infer_array_float_range() {
        let ft = FieldType::Array(Box::new(FieldType::Float));
        let f = make_field("scores", ft, vec![json!(0.1), json!(0.9), json!(0.5)]);
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Float(0.1));
        assert_eq!(max, toml::Value::Float(0.9));
    }

    #[test]
    fn infer_single_value() {
        let f = make_field("x", FieldType::Integer, vec![json!(42)]);
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Integer(42));
        assert_eq!(max, toml::Value::Integer(42));
    }

    #[test]
    fn infer_negative_integers() {
        let f = make_field(
            "delta",
            FieldType::Integer,
            vec![json!(-10), json!(5), json!(-3)],
        );
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Integer(-10));
        assert_eq!(max, toml::Value::Integer(5));
    }

    #[test]
    fn infer_string_field_returns_none() {
        let f = make_field("name", FieldType::String, vec![json!("hello")]);
        assert!(infer(&f).is_none());
    }

    #[test]
    fn infer_boolean_field_returns_none() {
        let f = make_field("flag", FieldType::Boolean, vec![json!(true)]);
        assert!(infer(&f).is_none());
    }

    #[test]
    fn infer_empty_values_returns_none() {
        let f = make_field("x", FieldType::Integer, vec![]);
        assert!(infer(&f).is_none());
    }

    #[test]
    fn infer_non_numeric_values_skipped() {
        // Mixed values — only numeric ones count (null and strings filtered out)
        let f = make_field(
            "x",
            FieldType::Integer,
            vec![json!(null), json!("oops"), json!(5)],
        );
        let (min, max) = infer(&f).unwrap();
        assert_eq!(min, toml::Value::Integer(5));
        assert_eq!(max, toml::Value::Integer(5));
    }
}
