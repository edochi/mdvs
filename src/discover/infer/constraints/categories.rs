//! Categorical inference — heuristic detection of categorical fields.

use crate::discover::field_type::FieldType;
use crate::discover::infer::InferredField;
use crate::schema::constraints::Constraints;
use serde_json::Value;

/// Apply the categorical heuristic to an inferred field and return constraints
/// if it qualifies. A field is categorical when:
/// 1. Its type supports categories (String, Integer, Array(String), Array(Integer))
/// 2. It has at most `max_categories` distinct values
/// 3. The average repetition (`occurrence_count / distinct_count`) ≥ `min_repetition`
pub fn infer(
    field: &InferredField,
    max_categories: usize,
    min_repetition: usize,
) -> Option<Constraints> {
    if !type_supports_categories(&field.field_type) {
        return None;
    }

    let distinct_count = field.distinct_values.len();
    if distinct_count == 0 || distinct_count > max_categories {
        return None;
    }

    if field.occurrence_count / distinct_count < min_repetition {
        return None;
    }

    let mut categories: Vec<toml::Value> = field
        .distinct_values
        .iter()
        .filter_map(json_to_toml_value)
        .collect();

    // Sort for deterministic output
    categories.sort_by(cmp_toml_values);

    Some(Constraints {
        categories: Some(categories),
    })
}

/// Check if a field type supports categorical constraints.
fn type_supports_categories(ft: &FieldType) -> bool {
    match ft {
        FieldType::String | FieldType::Integer => true,
        FieldType::Array(inner) => {
            matches!(inner.as_ref(), FieldType::String | FieldType::Integer)
        }
        _ => false,
    }
}

/// Convert a serde_json::Value to a toml::Value for category storage.
fn json_to_toml_value(val: &Value) -> Option<toml::Value> {
    match val {
        Value::String(s) => Some(toml::Value::String(s.clone())),
        Value::Number(n) => n.as_i64().map(toml::Value::Integer),
        _ => None,
    }
}

/// Compare two toml::Values for sorting (strings alphabetically, integers numerically).
fn cmp_toml_values(a: &toml::Value, b: &toml::Value) -> std::cmp::Ordering {
    match (a, b) {
        (toml::Value::String(a), toml::Value::String(b)) => a.cmp(b),
        (toml::Value::Integer(a), toml::Value::Integer(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_field(
        name: &str,
        ft: FieldType,
        distinct: Vec<Value>,
        occurrences: usize,
    ) -> InferredField {
        InferredField {
            name: name.into(),
            field_type: ft,
            files: vec![],
            allowed: vec![],
            required: vec![],
            nullable: false,
            distinct_values: distinct,
            occurrence_count: occurrences,
        }
    }

    #[test]
    fn categorical_string_field_inferred() {
        let f = make_field(
            "status",
            FieldType::String,
            vec![json!("draft"), json!("published"), json!("archived")],
            30,
        );
        let c = infer(&f, 10, 2).unwrap();
        let cats = c.categories.unwrap();
        assert_eq!(cats.len(), 3);
        assert_eq!(cats[0], toml::Value::String("archived".into()));
        assert_eq!(cats[1], toml::Value::String("draft".into()));
        assert_eq!(cats[2], toml::Value::String("published".into()));
    }

    #[test]
    fn categorical_integer_field_inferred() {
        let f = make_field(
            "priority",
            FieldType::Integer,
            vec![json!(1), json!(2), json!(3)],
            18,
        );
        let c = infer(&f, 10, 2).unwrap();
        let cats = c.categories.unwrap();
        assert_eq!(cats.len(), 3);
        assert_eq!(cats[0], toml::Value::Integer(1));
        assert_eq!(cats[1], toml::Value::Integer(2));
        assert_eq!(cats[2], toml::Value::Integer(3));
    }

    #[test]
    fn non_categorical_high_cardinality() {
        let distinct: Vec<Value> = (0..20).map(|i| json!(format!("title_{i}"))).collect();
        let f = make_field("title", FieldType::String, distinct, 40);
        assert!(infer(&f, 10, 2).is_none());
    }

    #[test]
    fn non_categorical_low_repetition() {
        let f = make_field(
            "author",
            FieldType::String,
            vec![
                json!("alice"),
                json!("bob"),
                json!("carol"),
                json!("dave"),
                json!("eve"),
            ],
            5,
        );
        assert!(infer(&f, 10, 2).is_none());
    }

    #[test]
    fn categorical_array_element_level() {
        let f = make_field(
            "tags",
            FieldType::Array(Box::new(FieldType::String)),
            vec![json!("rust"), json!("python"), json!("go")],
            30,
        );
        let c = infer(&f, 10, 2).unwrap();
        let cats = c.categories.unwrap();
        assert_eq!(cats.len(), 3);
        assert_eq!(cats[0], toml::Value::String("go".into()));
    }

    #[test]
    fn non_categorical_boolean_skipped() {
        let f = make_field(
            "draft",
            FieldType::Boolean,
            vec![json!(true), json!(false)],
            20,
        );
        assert!(infer(&f, 10, 2).is_none());
    }

    #[test]
    fn non_categorical_float_skipped() {
        let f = make_field("score", FieldType::Float, vec![json!(1.5), json!(2.5)], 20);
        assert!(infer(&f, 10, 2).is_none());
    }

    #[test]
    fn categories_sorted_deterministically() {
        let f = make_field(
            "status",
            FieldType::String,
            vec![json!("zebra"), json!("alpha"), json!("middle")],
            30,
        );
        let c = infer(&f, 10, 2).unwrap();
        let cats = c.categories.unwrap();
        assert_eq!(cats[0], toml::Value::String("alpha".into()));
        assert_eq!(cats[1], toml::Value::String("middle".into()));
        assert_eq!(cats[2], toml::Value::String("zebra".into()));
    }

    #[test]
    fn edge_case_single_value_all_files() {
        let f = make_field("lang", FieldType::String, vec![json!("en")], 50);
        let c = infer(&f, 10, 2).unwrap();
        assert_eq!(c.categories.unwrap().len(), 1);
    }

    #[test]
    fn edge_case_at_threshold_boundary() {
        let f = make_field(
            "level",
            FieldType::Integer,
            vec![json!(1), json!(2), json!(3)],
            6,
        );
        assert!(infer(&f, 3, 2).is_some());
        assert!(infer(&f, 2, 2).is_none());

        let f2 = make_field(
            "level",
            FieldType::Integer,
            vec![json!(1), json!(2), json!(3)],
            5,
        );
        assert!(infer(&f2, 3, 2).is_none());
    }
}
