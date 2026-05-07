//! Two-layer constraint architecture for field value validation.
//!
//! **Serde layer**: [`Constraints`] — flat struct mapping to `[fields.field.constraints]` in TOML.
//! **Behavior layer**: [`ConstraintKind`] — enum for structured dispatch of type applicability,
//! value validation, and pairwise compatibility checks.
//!
//! Each constraint kind has its own submodule with validation and inference logic.
//! [`Constraints::active()`] bridges the two layers, and [`Constraints::validate_config()`]
//! runs the full resolver (self-validation + pairwise compatibility).

mod categories;
mod range;

use crate::discover::field_type::FieldType;
use crate::output::ViolationKind;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Serde layer
// ---------------------------------------------------------------------------

/// Flat serde layer for `[fields.field.constraints]` in TOML.
/// Each constraint kind is an `Option` field — absent means unconstrained.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Constraints {
    /// Restrict values to an enumerated set. Applies to String, Integer,
    /// Array(String), and Array(Integer) fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<toml::Value>>,
    /// Minimum value (inclusive). Applies to Integer, Float,
    /// Array(Integer), and Array(Float) fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<toml::Value>,
    /// Maximum value (inclusive). Applies to Integer, Float,
    /// Array(Integer), and Array(Float) fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<toml::Value>,
    // future (TODO-0010): min_length, max_length
    // future (TODO-0145): pattern
}

// ---------------------------------------------------------------------------
// Behavior layer
// ---------------------------------------------------------------------------

/// Behavior dispatch layer. Each variant encapsulates one constraint's
/// validation logic: which types it applies to and how it checks values.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConstraintKind {
    /// Value must be one of the listed categories.
    Categories(Vec<toml::Value>),
    /// Value must fall within [min, max] (inclusive).
    Range {
        min: Option<toml::Value>,
        max: Option<toml::Value>,
    },
    // future (TODO-0010): Length { min: Option<usize>, max: Option<usize> }
    // future (TODO-0145): Pattern(String)
}

/// A constraint violation for a single value check.
/// Maps to `FieldViolation::rule` and `ViolatingFile::detail` when integrated
/// with the check pipeline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConstraintViolation {
    /// Human-readable rule description, e.g. `categories = ["draft", "published"]`.
    pub rule: String,
    /// Detail about the specific violation, e.g. `got "pending"`.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// Bridge
// ---------------------------------------------------------------------------

impl Constraints {
    /// Convert the flat serde fields into structured [`ConstraintKind`] variants.
    /// Returns only the constraints that are actually set (non-`None`).
    pub(crate) fn active(&self) -> Vec<ConstraintKind> {
        let mut result = Vec::new();
        if let Some(cats) = &self.categories {
            result.push(ConstraintKind::Categories(cats.clone()));
        }
        if self.min.is_some() || self.max.is_some() {
            result.push(ConstraintKind::Range {
                min: self.min.clone(),
                max: self.max.clone(),
            });
        }
        // future: check min_length/max_length, pattern
        result
    }
}

// ---------------------------------------------------------------------------
// Resolver
// ---------------------------------------------------------------------------

impl Constraints {
    /// Validate all active constraints against a field's type.
    ///
    /// Two-phase validation:
    /// 1. Each constraint checks it is applicable to `field_type` and well-formed.
    /// 2. Each pair of constraints checks mutual compatibility.
    ///
    /// Returns a list of human-readable error messages (empty = valid).
    pub(crate) fn validate_config(&self, field_name: &str, field_type: &FieldType) -> Vec<String> {
        let active = self.active();
        let mut errors = Vec::new();

        // Phase 1: self-validation
        for c in &active {
            if let Some(err) = c.validate_for_type(field_name, field_type) {
                errors.push(err);
            }
        }

        // Phase 2: pairwise compatibility
        for i in 0..active.len() {
            for j in (i + 1)..active.len() {
                if let Some(err) = active[i].conflicts_with(&active[j], field_name, field_type) {
                    errors.push(err);
                }
            }
        }

        errors
    }
}

// ---------------------------------------------------------------------------
// ConstraintKind — dispatch to per-kind submodules
// ---------------------------------------------------------------------------

impl ConstraintKind {
    /// Check that this constraint is applicable to the given field type
    /// and that its configuration values are well-formed.
    pub(crate) fn validate_for_type(
        &self,
        field_name: &str,
        field_type: &FieldType,
    ) -> Option<String> {
        match self {
            ConstraintKind::Categories(values) => {
                categories::validate_for_type(field_name, field_type, values)
            }
            ConstraintKind::Range { min, max } => {
                range::validate_for_type(field_name, field_type, min, max)
            }
        }
    }

    /// Check a frontmatter value against this constraint.
    pub(crate) fn validate_value(
        &self,
        value: &serde_json::Value,
        field_type: &FieldType,
    ) -> Option<ConstraintViolation> {
        match self {
            ConstraintKind::Categories(cats) => categories::validate_value(value, field_type, cats),
            ConstraintKind::Range { min, max } => {
                range::validate_value(value, field_type, min, max)
            }
        }
    }

    /// Return the violation kind for this constraint type.
    pub(crate) fn violation_kind(&self) -> ViolationKind {
        match self {
            ConstraintKind::Categories(_) => ViolationKind::InvalidCategory,
            ConstraintKind::Range { .. } => ViolationKind::OutOfRange,
        }
    }

    /// Check whether this constraint conflicts with another on the same field.
    pub(crate) fn conflicts_with(
        &self,
        other: &ConstraintKind,
        field_name: &str,
        _field_type: &FieldType,
    ) -> Option<String> {
        match (self, other) {
            (ConstraintKind::Categories(_), ConstraintKind::Categories(_)) => Some(format!(
                "field '{field_name}': duplicate categories constraint"
            )),
            (ConstraintKind::Range { .. }, ConstraintKind::Range { .. }) => {
                Some(format!("field '{field_name}': duplicate range constraint"))
            }
            (ConstraintKind::Categories(_), ConstraintKind::Range { .. })
            | (ConstraintKind::Range { .. }, ConstraintKind::Categories(_)) => Some(format!(
                "field '{field_name}': categories and range constraints are mutually exclusive"
            )),
        }
    }
}

// ===========================================================================
// Tests — serde, bridge, resolver, conflicts (cross-constraint concerns)
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn str_cats(vals: &[&str]) -> Vec<toml::Value> {
        vals.iter()
            .map(|s| toml::Value::String(s.to_string()))
            .collect()
    }

    fn int_cats(vals: &[i64]) -> Vec<toml::Value> {
        vals.iter().map(|&n| toml::Value::Integer(n)).collect()
    }

    fn cats_constraint(cats: Vec<toml::Value>) -> Constraints {
        Constraints {
            categories: Some(cats),
            ..Default::default()
        }
    }

    fn cats_kind(cats: Vec<toml::Value>) -> ConstraintKind {
        ConstraintKind::Categories(cats)
    }

    // -----------------------------------------------------------------------
    // Serde roundtrip
    // -----------------------------------------------------------------------

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Wrapper {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        constraints: Option<Constraints>,
    }

    #[test]
    fn serde_empty_constraints_roundtrip() {
        let c = Constraints::default();
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: Constraints = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn serde_string_categories_roundtrip() {
        let c = cats_constraint(str_cats(&["draft", "published", "archived"]));
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: Constraints = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn serde_integer_categories_roundtrip() {
        let c = cats_constraint(int_cats(&[1, 2, 3]));
        let toml_str = toml::to_string(&c).unwrap();
        let parsed: Constraints = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, c);
    }

    #[test]
    fn serde_absent_categories_omitted() {
        let c = Constraints::default();
        let toml_str = toml::to_string(&c).unwrap();
        assert!(!toml_str.contains("categories"));
    }

    #[test]
    fn serde_in_wrapper_context() {
        let toml_str = r#"
[constraints]
categories = ["draft", "published"]
"#;
        let parsed: Wrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(
            parsed.constraints,
            Some(cats_constraint(str_cats(&["draft", "published"])))
        );

        let parsed_empty: Wrapper = toml::from_str("").unwrap();
        assert_eq!(parsed_empty.constraints, None);
    }

    // -----------------------------------------------------------------------
    // active()
    // -----------------------------------------------------------------------

    #[test]
    fn active_empty_constraints() {
        assert!(Constraints::default().active().is_empty());
    }

    #[test]
    fn active_with_categories() {
        let cats = str_cats(&["a", "b"]);
        let c = cats_constraint(cats.clone());
        assert_eq!(c.active(), vec![ConstraintKind::Categories(cats)]);
    }

    // -----------------------------------------------------------------------
    // validate_config integration
    // -----------------------------------------------------------------------

    #[test]
    fn validate_config_valid_string_categories() {
        let c = cats_constraint(str_cats(&["a", "b"]));
        assert!(c.validate_config("f", &FieldType::String).is_empty());
    }

    #[test]
    fn validate_config_invalid_type_for_categories() {
        let c = cats_constraint(str_cats(&["a", "b"]));
        let errors = c.validate_config("f", &FieldType::Boolean);
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_config_mismatched_category_values() {
        let c = cats_constraint(int_cats(&[1, 2]));
        let errors = c.validate_config("f", &FieldType::String);
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_config_no_constraints() {
        let c = Constraints::default();
        assert!(c.validate_config("f", &FieldType::String).is_empty());
    }

    // -----------------------------------------------------------------------
    // conflicts_with
    // -----------------------------------------------------------------------

    #[test]
    fn conflicts_with_duplicate_categories() {
        let a = cats_kind(str_cats(&["a"]));
        let b = cats_kind(str_cats(&["b"]));
        let err = a.conflicts_with(&b, "f", &FieldType::String).unwrap();
        assert!(err.contains("duplicate"));
    }
}
