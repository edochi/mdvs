//! Preprocessor pipeline run before jsonschema validation.
//!
//! Three-stage architecture (per TODO-0149 design):
//! - Stage 1 (field-name, global): no built-ins in v0; framework only.
//! - Stage 2 (per-field value): `coerce_to_string`, `widen_int_to_float`.
//! - Stage 3 (per-document, global): no built-ins in v0; framework only.
//!
//! Stages are configured per `[[fields.field]]` (Stage 2) or in a future
//! top-level `[preprocess]` section (Stages 1 + 3). Inference auto-populates
//! Stage 2 entries based on observed type-widening events; users can also
//! configure them manually.

use crate::discover::field_type::FieldType;
use crate::schema::config::{MdvsToml, TomlField};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;

/// Per-field value preprocessor.
///
/// `coerce_to_string` serializes non-string non-null values via `Value::to_string()`
/// (matching the build-time JSON serialization in storage.rs).
///
/// `widen_int_to_float` converts integer-backed numbers to f64-backed numbers
/// for fields typed `Float` or `Array(Float)` that received integer values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValueStage {
    /// Serialize non-string non-null values to JSON-stringified form.
    /// Auto-inferred when widening landed on `String` due to mixed scalar types.
    CoerceToString,
    /// Convert integer-backed numbers to f64-backed numbers.
    /// Auto-inferred when widening landed on `Float` due to int+float mix.
    WidenIntToFloat,
}

impl ValueStage {
    /// Whether this stage can transform values for the given field type.
    /// Used both for runtime dispatch and for config-time validation.
    ///
    /// Adding a new variant fails compilation here until handled — exhaustive
    /// match enforces per-variant accountability.
    pub fn applies_to(&self, ft: &FieldType) -> bool {
        match self {
            ValueStage::CoerceToString => match ft {
                FieldType::String => true,
                FieldType::Array(inner) => matches!(**inner, FieldType::String),
                _ => false,
            },
            ValueStage::WidenIntToFloat => match ft {
                FieldType::Float => true,
                FieldType::Array(inner) => matches!(**inner, FieldType::Float),
                _ => false,
            },
        }
    }

    /// Human-readable description of valid field types, for error messages.
    pub fn applicable_types(&self) -> &'static str {
        match self {
            ValueStage::CoerceToString => "String, Array(String)",
            ValueStage::WidenIntToFloat => "Float, Array(Float)",
        }
    }

    /// Apply this stage to a value. Returns `None` if no transformation
    /// applied — the caller keeps the input as-is.
    fn apply(&self, value: &Value, field_type: &FieldType) -> Option<Value> {
        match self {
            ValueStage::CoerceToString => coerce_to_string(value, field_type),
            ValueStage::WidenIntToFloat => widen_int_to_float(value, field_type),
        }
    }
}

impl std::fmt::Display for ValueStage {
    /// Renders the snake_case form used in `mdvs.toml`. Matches the serde
    /// rename rule — error messages and toml stay consistent.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            ValueStage::CoerceToString => "coerce_to_string",
            ValueStage::WidenIntToFloat => "widen_int_to_float",
        };
        f.write_str(name)
    }
}

/// Field-name preprocessor (Stage 1). Empty in v0; future variants:
/// `Lua(LuaScript)`, `Rename { from, to }`, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FieldNameStage {}

/// Per-document preprocessor (Stage 3). Empty in v0; future variants:
/// `Lua(LuaScript)`, `Drop { fields: [...] }`, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DocumentStage {}

/// Compiled preprocessor pipeline for a config. Built once per `validate()` call.
pub(crate) struct Pipeline {
    per_field: HashMap<String, Vec<ValueStage>>,
}

impl Pipeline {
    /// Build the pipeline from `[[fields.field]] preprocess = [...]` entries.
    pub(crate) fn for_config(config: &MdvsToml) -> Self {
        let per_field = config
            .fields
            .field
            .iter()
            .filter(|f| !f.preprocess.is_empty())
            .map(|f| (f.name.clone(), f.preprocess.clone()))
            .collect();
        Pipeline { per_field }
    }

    /// Apply the field's Stage-2 preprocessors in declared order.
    /// Returns `Cow::Borrowed(value)` when no transformation occurs.
    pub(crate) fn apply_to_value<'a>(&self, field: &TomlField, value: &'a Value) -> Cow<'a, Value> {
        let stages = match self.per_field.get(field.name.as_str()) {
            Some(s) => s,
            None => return Cow::Borrowed(value),
        };

        let field_type = match FieldType::try_from(&field.field_type) {
            Ok(ft) => ft,
            Err(_) => return Cow::Borrowed(value),
        };

        let mut current: Cow<'a, Value> = Cow::Borrowed(value);
        for stage in stages {
            if let Some(v) = stage.apply(current.as_ref(), &field_type) {
                current = Cow::Owned(v);
            }
        }
        current
    }
}

// ============================================================================
// Stage 2 built-ins
// ============================================================================

/// Serialize non-string non-null values to JSON-stringified form. Operates on
/// the field-level value when `field_type` is `String`, or per-element when
/// it's `Array(String)`.
fn coerce_to_string(value: &Value, field_type: &FieldType) -> Option<Value> {
    match (field_type, value) {
        (FieldType::String, v) if !v.is_string() && !v.is_null() => {
            Some(Value::String(v.to_string()))
        }
        (FieldType::Array(inner), Value::Array(arr)) if matches!(**inner, FieldType::String) => {
            let coerced: Vec<Value> = arr
                .iter()
                .map(|elem| {
                    if !elem.is_string() && !elem.is_null() {
                        Value::String(elem.to_string())
                    } else {
                        elem.clone()
                    }
                })
                .collect();
            Some(Value::Array(coerced))
        }
        _ => None,
    }
}

/// Convert integer-backed numbers to f64-backed numbers for Float / Array(Float)
/// fields. Returns `None` when no conversion is needed.
fn widen_int_to_float(value: &Value, field_type: &FieldType) -> Option<Value> {
    match (field_type, value) {
        (FieldType::Float, Value::Number(n)) if n.is_i64() => n
            .as_i64()
            .and_then(|i| serde_json::Number::from_f64(i as f64).map(Value::Number)),
        (FieldType::Array(inner), Value::Array(arr)) if matches!(**inner, FieldType::Float) => {
            let widened: Vec<Value> = arr
                .iter()
                .map(|elem| match elem {
                    Value::Number(n) if n.is_i64() => n
                        .as_i64()
                        .and_then(|i| serde_json::Number::from_f64(i as f64))
                        .map(Value::Number)
                        .unwrap_or_else(|| elem.clone()),
                    _ => elem.clone(),
                })
                .collect();
            Some(Value::Array(widened))
        }
        _ => None,
    }
}

// ============================================================================
// Strict subtype prechecks (dual of Stage-2 preprocessors)
// ============================================================================

/// Strict-subtype precheck: reject values whose JSON subtype is wrong for the
/// field type when the relevant opt-in preprocessor is absent.
///
/// JSON Schema can't see the difference between `Value::Number(5)` (i64-backed)
/// and `Value::Number(5.0)` (f64-backed) — both match `"number"`. So enforcing
/// strict Float (reject integers) has to live in Rust, before jsonschema.
///
/// Returns `Some(detail)` when the value must be rejected. The caller emits a
/// `ViolationKind::WrongType` violation and skips both the preprocessor pipeline
/// and the jsonschema validator for this value.
///
/// `CoerceToString` is not handled here — `{"type": "string"}` rejects non-strings
/// natively, so jsonschema covers that side of strictness.
pub(crate) fn strict_subtype_check(
    field: &TomlField,
    field_type: &FieldType,
    value: &Value,
) -> Option<String> {
    // Float strict: reject integer-backed numbers unless widen_int_to_float
    // is opted in.
    if matches!(field_type, FieldType::Float)
        && !field.preprocess.contains(&ValueStage::WidenIntToFloat)
        && matches!(value, Value::Number(n) if n.is_i64() || n.is_u64())
    {
        return Some("got Integer".to_string());
    }

    // Array(Float) strict: reject if any element is integer-backed unless
    // widen_int_to_float is opted in.
    if let FieldType::Array(inner) = field_type
        && matches!(**inner, FieldType::Float)
        && !field.preprocess.contains(&ValueStage::WidenIntToFloat)
        && let Value::Array(arr) = value
    {
        let bad = arr.iter().enumerate().find_map(|(i, elem)| match elem {
            Value::Number(n) if n.is_i64() || n.is_u64() => Some(i),
            _ => None,
        });
        if let Some(i) = bad {
            return Some(format!("got Integer at index {i}"));
        }
    }

    None
}

// ============================================================================
// Inference: derive Stage-2 preprocessors from observed widening events
// ============================================================================

/// Compute the Stage-2 preprocessors implied by inference observations.
///
/// `observed` is the set of non-null observation types seen across all files
/// for this field. `final_type` is the widened result.
///
/// Triggers:
/// - `CoerceToString` when `final_type` collapsed to String due to mixed
///   non-string observations (or `Array(String)` saw mixed inner types).
/// - `WidenIntToFloat` when `final_type` is Float with at least one Integer
///   observation (or `Array(Float)` saw any `Array(Integer)` observation).
pub fn infer_value_stages(observed: &[FieldType], final_type: &FieldType) -> Vec<ValueStage> {
    let mut stages = Vec::new();

    if needs_coerce_to_string(observed, final_type) {
        stages.push(ValueStage::CoerceToString);
    }
    if needs_widen_int_to_float(observed, final_type) {
        stages.push(ValueStage::WidenIntToFloat);
    }

    stages
}

fn needs_coerce_to_string(observed: &[FieldType], final_type: &FieldType) -> bool {
    match final_type {
        FieldType::String => observed.iter().any(|t| !matches!(t, FieldType::String)),
        FieldType::Array(inner) if matches!(**inner, FieldType::String) => {
            observed.iter().any(|t| match t {
                // Array of non-string elements
                FieldType::Array(i) => !matches!(**i, FieldType::String),
                // Non-array observation (scalar/object) on an Array(String) field
                _ => true,
            })
        }
        _ => false,
    }
}

fn needs_widen_int_to_float(observed: &[FieldType], final_type: &FieldType) -> bool {
    match final_type {
        FieldType::Float => observed.iter().any(|t| matches!(t, FieldType::Integer)),
        FieldType::Array(inner) if matches!(**inner, FieldType::Float) => {
            observed.iter().any(|t| match t {
                FieldType::Array(i) => matches!(**i, FieldType::Integer),
                _ => false,
            })
        }
        _ => false,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::shared::FieldTypeSerde;
    use serde_json::json;

    fn string_field(name: &str, preprocess: Vec<ValueStage>) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess,
        }
    }

    fn float_field(name: &str, preprocess: Vec<ValueStage>) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("Float".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess,
        }
    }

    fn array_string_field(name: &str, preprocess: Vec<ValueStage>) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess,
        }
    }

    // -----------------------------------------------------------------------
    // ValueStage::applies_to / applicable_types
    // -----------------------------------------------------------------------

    #[test]
    fn coerce_to_string_applies_to_string_and_array_string() {
        assert!(ValueStage::CoerceToString.applies_to(&FieldType::String));
        assert!(
            ValueStage::CoerceToString.applies_to(&FieldType::Array(Box::new(FieldType::String)))
        );
    }

    #[test]
    fn coerce_to_string_does_not_apply_to_other_types() {
        assert!(!ValueStage::CoerceToString.applies_to(&FieldType::Integer));
        assert!(!ValueStage::CoerceToString.applies_to(&FieldType::Float));
        assert!(!ValueStage::CoerceToString.applies_to(&FieldType::Boolean));
        assert!(
            !ValueStage::CoerceToString.applies_to(&FieldType::Array(Box::new(FieldType::Integer)))
        );
    }

    #[test]
    fn widen_int_to_float_applies_to_float_and_array_float() {
        assert!(ValueStage::WidenIntToFloat.applies_to(&FieldType::Float));
        assert!(
            ValueStage::WidenIntToFloat.applies_to(&FieldType::Array(Box::new(FieldType::Float)))
        );
    }

    #[test]
    fn widen_int_to_float_does_not_apply_to_other_types() {
        assert!(!ValueStage::WidenIntToFloat.applies_to(&FieldType::String));
        assert!(!ValueStage::WidenIntToFloat.applies_to(&FieldType::Integer));
        assert!(!ValueStage::WidenIntToFloat.applies_to(&FieldType::Boolean));
        assert!(
            !ValueStage::WidenIntToFloat.applies_to(&FieldType::Array(Box::new(FieldType::String)))
        );
    }

    #[test]
    fn applicable_types_strings_describe_each_variant() {
        // Tied to user-facing error messages — confirm the descriptions stay
        // accurate as variants are added.
        assert_eq!(
            ValueStage::CoerceToString.applicable_types(),
            "String, Array(String)"
        );
        assert_eq!(
            ValueStage::WidenIntToFloat.applicable_types(),
            "Float, Array(Float)"
        );
    }

    #[test]
    fn display_renders_snake_case_for_each_variant() {
        // Display matches the serde rename — error messages and toml stay in sync.
        assert_eq!(
            format!("{}", ValueStage::CoerceToString),
            "coerce_to_string"
        );
        assert_eq!(
            format!("{}", ValueStage::WidenIntToFloat),
            "widen_int_to_float"
        );
    }

    #[test]
    fn apply_method_dispatches_per_variant() {
        // Smoke test: the inherent `apply` method matches the underlying
        // free-function behavior.
        let r = ValueStage::CoerceToString.apply(&json!(42), &FieldType::String);
        assert_eq!(r, Some(json!("42")));

        let r = ValueStage::WidenIntToFloat.apply(&json!(42), &FieldType::Float);
        assert_eq!(r.and_then(|v| v.as_f64()), Some(42.0));
    }

    // -----------------------------------------------------------------------
    // coerce_to_string
    // -----------------------------------------------------------------------

    #[test]
    fn coerce_to_string_preserves_string_value() {
        let r = coerce_to_string(&json!("hello"), &FieldType::String);
        assert!(r.is_none()); // no-op
    }

    #[test]
    fn coerce_to_string_serializes_integer() {
        let r = coerce_to_string(&json!(42), &FieldType::String).unwrap();
        assert_eq!(r, json!("42"));
    }

    #[test]
    fn coerce_to_string_serializes_array_for_string_field() {
        let r = coerce_to_string(&json!(["internal"]), &FieldType::String).unwrap();
        assert_eq!(r, json!(r#"["internal"]"#));
    }

    #[test]
    fn coerce_to_string_preserves_null() {
        let r = coerce_to_string(&json!(null), &FieldType::String);
        assert!(r.is_none());
    }

    #[test]
    fn coerce_to_string_per_element_for_array_string() {
        let arr_str = FieldType::Array(Box::new(FieldType::String));
        let r = coerce_to_string(&json!(["ok", 42, true]), &arr_str).unwrap();
        assert_eq!(r, json!(["ok", "42", "true"]));
    }

    #[test]
    fn coerce_to_string_no_op_for_integer_field() {
        let r = coerce_to_string(&json!(42), &FieldType::Integer);
        assert!(r.is_none());
    }

    // -----------------------------------------------------------------------
    // widen_int_to_float
    // -----------------------------------------------------------------------

    #[test]
    fn widen_int_to_float_basic() {
        let r = widen_int_to_float(&json!(42), &FieldType::Float).unwrap();
        // The serde_json::Number internal representation differs but value is same
        assert_eq!(r.as_f64(), Some(42.0));
    }

    #[test]
    fn widen_int_to_float_no_op_on_float() {
        let r = widen_int_to_float(&json!(2.5), &FieldType::Float);
        assert!(r.is_none());
    }

    #[test]
    fn widen_int_to_float_no_op_on_string() {
        let r = widen_int_to_float(&json!("hello"), &FieldType::Float);
        assert!(r.is_none());
    }

    #[test]
    fn widen_int_to_float_per_element_for_array_float() {
        let arr_float = FieldType::Array(Box::new(FieldType::Float));
        let r = widen_int_to_float(&json!([1, 2.5, 3]), &arr_float).unwrap();
        let arr = r.as_array().unwrap();
        assert_eq!(arr[0].as_f64(), Some(1.0));
        assert_eq!(arr[1].as_f64(), Some(2.5));
        assert_eq!(arr[2].as_f64(), Some(3.0));
    }

    // -----------------------------------------------------------------------
    // Pipeline
    // -----------------------------------------------------------------------

    #[test]
    fn pipeline_no_preprocess_returns_borrowed() {
        use crate::schema::config::{FieldsConfig, UpdateConfig};
        use crate::schema::shared::ScanConfig;
        let toml = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![string_field("title", vec![])],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        let pipeline = Pipeline::for_config(&toml);
        let v = json!("hello");
        let result = pipeline.apply_to_value(&toml.fields.field[0], &v);
        assert!(matches!(result, Cow::Borrowed(_)));
    }

    #[test]
    fn pipeline_applies_coerce_to_string() {
        use crate::schema::config::{FieldsConfig, UpdateConfig};
        use crate::schema::shared::ScanConfig;
        let toml = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![string_field("title", vec![ValueStage::CoerceToString])],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        let pipeline = Pipeline::for_config(&toml);
        let v = json!(42);
        let result = pipeline.apply_to_value(&toml.fields.field[0], &v);
        assert_eq!(*result, json!("42"));
    }

    #[test]
    fn pipeline_applies_widen_int_to_float() {
        use crate::schema::config::{FieldsConfig, UpdateConfig};
        use crate::schema::shared::ScanConfig;
        let toml = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![float_field("score", vec![ValueStage::WidenIntToFloat])],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        let pipeline = Pipeline::for_config(&toml);
        let v = json!(42);
        let result = pipeline.apply_to_value(&toml.fields.field[0], &v);
        assert_eq!(result.as_f64(), Some(42.0));
    }

    #[test]
    fn pipeline_array_string_per_element() {
        use crate::schema::config::{FieldsConfig, UpdateConfig};
        use crate::schema::shared::ScanConfig;
        let toml = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![array_string_field("tags", vec![ValueStage::CoerceToString])],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        let pipeline = Pipeline::for_config(&toml);
        let v = json!(["ok", 42]);
        let result = pipeline.apply_to_value(&toml.fields.field[0], &v);
        assert_eq!(*result, json!(["ok", "42"]));
    }

    // -----------------------------------------------------------------------
    // infer_value_stages
    // -----------------------------------------------------------------------

    #[test]
    fn infer_uniform_string_no_preprocess() {
        let stages = infer_value_stages(
            &[FieldType::String, FieldType::String, FieldType::String],
            &FieldType::String,
        );
        assert!(stages.is_empty());
    }

    #[test]
    fn infer_int_string_widens_with_coerce() {
        let stages =
            infer_value_stages(&[FieldType::Integer, FieldType::String], &FieldType::String);
        assert_eq!(stages, vec![ValueStage::CoerceToString]);
    }

    #[test]
    fn infer_int_float_widens_with_widen_int_to_float() {
        let stages = infer_value_stages(&[FieldType::Integer, FieldType::Float], &FieldType::Float);
        assert_eq!(stages, vec![ValueStage::WidenIntToFloat]);
    }

    #[test]
    fn infer_array_int_array_float_widens_with_widen() {
        let observed = vec![
            FieldType::Array(Box::new(FieldType::Integer)),
            FieldType::Array(Box::new(FieldType::Float)),
        ];
        let final_type = FieldType::Array(Box::new(FieldType::Float));
        let stages = infer_value_stages(&observed, &final_type);
        assert_eq!(stages, vec![ValueStage::WidenIntToFloat]);
    }

    #[test]
    fn infer_string_scalar_plus_array_widens_with_coerce() {
        let observed = vec![
            FieldType::String,
            FieldType::Array(Box::new(FieldType::String)),
        ];
        let stages = infer_value_stages(&observed, &FieldType::String);
        assert_eq!(stages, vec![ValueStage::CoerceToString]);
    }

    #[test]
    fn infer_uniform_array_string_no_preprocess() {
        let observed = vec![FieldType::Array(Box::new(FieldType::String))];
        let final_type = FieldType::Array(Box::new(FieldType::String));
        let stages = infer_value_stages(&observed, &final_type);
        assert!(stages.is_empty());
    }

    #[test]
    fn infer_array_mixed_inner_type_coerces() {
        // Array(Integer) + Array(String) collapses to Array(String) (via inner widen).
        let observed = vec![
            FieldType::Array(Box::new(FieldType::Integer)),
            FieldType::Array(Box::new(FieldType::String)),
        ];
        let final_type = FieldType::Array(Box::new(FieldType::String));
        let stages = infer_value_stages(&observed, &final_type);
        assert_eq!(stages, vec![ValueStage::CoerceToString]);
    }

    // -----------------------------------------------------------------------
    // strict_subtype_check
    // -----------------------------------------------------------------------

    fn array_float_field(name: &str, preprocess: Vec<ValueStage>) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Float".into())),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess,
        }
    }

    #[test]
    fn strict_check_rejects_integer_on_strict_float() {
        let field = float_field("score", vec![]);
        let result = strict_subtype_check(&field, &FieldType::Float, &json!(5));
        assert_eq!(result.as_deref(), Some("got Integer"));
    }

    #[test]
    fn strict_check_accepts_integer_when_widen_int_to_float_set() {
        let field = float_field("score", vec![ValueStage::WidenIntToFloat]);
        let result = strict_subtype_check(&field, &FieldType::Float, &json!(5));
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_accepts_float_on_strict_float() {
        let field = float_field("score", vec![]);
        let result = strict_subtype_check(&field, &FieldType::Float, &json!(5.0));
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_ignores_null_on_strict_float() {
        // Null handling is jsonschema's job (NullNotAllowed). Precheck stays out.
        let field = float_field("score", vec![]);
        let result = strict_subtype_check(&field, &FieldType::Float, &Value::Null);
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_ignores_string_on_strict_float() {
        // Wrong primitive type is jsonschema's job (WrongType from "number").
        let field = float_field("score", vec![]);
        let result = strict_subtype_check(&field, &FieldType::Float, &json!("hi"));
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_rejects_integer_element_in_strict_array_float() {
        let field = array_float_field("scores", vec![]);
        let arr_type = FieldType::Array(Box::new(FieldType::Float));
        let result = strict_subtype_check(&field, &arr_type, &json!([1.0, 2, 3.0]));
        assert_eq!(result.as_deref(), Some("got Integer at index 1"));
    }

    #[test]
    fn strict_check_accepts_integer_element_in_array_float_with_widen() {
        let field = array_float_field("scores", vec![ValueStage::WidenIntToFloat]);
        let arr_type = FieldType::Array(Box::new(FieldType::Float));
        let result = strict_subtype_check(&field, &arr_type, &json!([1.0, 2, 3.0]));
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_accepts_uniform_float_array() {
        let field = array_float_field("scores", vec![]);
        let arr_type = FieldType::Array(Box::new(FieldType::Float));
        let result = strict_subtype_check(&field, &arr_type, &json!([1.0, 2.5, 3.0]));
        assert!(result.is_none());
    }

    #[test]
    fn strict_check_does_not_apply_to_non_float_fields() {
        // String, Integer, Boolean, Array(String) — no strict subtype check.
        let s = string_field("title", vec![]);
        assert!(strict_subtype_check(&s, &FieldType::String, &json!(5)).is_none());

        let arr_s = array_string_field("tags", vec![]);
        let arr_string_type = FieldType::Array(Box::new(FieldType::String));
        assert!(strict_subtype_check(&arr_s, &arr_string_type, &json!([1, 2])).is_none());
    }
}
