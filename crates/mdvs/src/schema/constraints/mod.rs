//! Two-layer constraint architecture for field value validation.
//!
//! **Serde layer**: [`Constraints`] — flat struct mapping to `[fields.field.constraints]` in TOML.
//! **Behavior layer**: [`ConstraintKind`] — enum for config-time dispatch of type applicability
//! and pairwise compatibility checks.
//!
//! Per-value validation is delegated to `jsonschema` via the translator in
//! `schema/json_schema.rs`; the constraint module is only responsible for
//! config-time validation (whether a constraint is well-formed and applicable
//! to a field type) and inference.

mod categories;
mod length;
mod pattern;
mod range;

use crate::discover::field_type::FieldType;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Serde layer
// ---------------------------------------------------------------------------

/// Flat serde layer for `[fields.field.constraints]` in TOML.
/// Each constraint kind is an `Option` field — absent means unconstrained.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
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
    /// Minimum string length (inclusive). Applies to String and Array(String).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    /// Maximum string length (inclusive). Applies to String and Array(String).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    /// Regex pattern values must match. Applies to String and Array(String).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
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
    /// String length must fall within [min, max] (inclusive).
    Length { min: Option<u64>, max: Option<u64> },
    /// String must match the given regex pattern.
    Pattern(String),
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
        if self.min_length.is_some() || self.max_length.is_some() {
            result.push(ConstraintKind::Length {
                min: self.min_length,
                max: self.max_length,
            });
        }
        if let Some(pat) = &self.pattern {
            result.push(ConstraintKind::Pattern(pat.clone()));
        }
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
            ConstraintKind::Length { min, max } => {
                length::validate_for_type(field_name, field_type, *min, *max)
            }
            ConstraintKind::Pattern(pat) => pattern::validate_for_type(field_name, field_type, pat),
        }
    }

    /// Check whether this constraint conflicts with another on the same field.
    ///
    /// Categories is closed-set semantics — incompatible with any other
    /// narrowing constraint (anything else is redundant or contradictory).
    /// Range applies to numeric fields and Length/Pattern to string fields,
    /// so they're mutually disjoint via the type checker. Length and Pattern
    /// are complementary on the same string field and may coexist.
    pub(crate) fn conflicts_with(
        &self,
        other: &ConstraintKind,
        field_name: &str,
        _field_type: &FieldType,
    ) -> Option<String> {
        use ConstraintKind::*;
        match (self, other) {
            (Categories(_), Categories(_)) => Some(format!(
                "field '{field_name}': duplicate categories constraint"
            )),
            (Range { .. }, Range { .. }) => {
                Some(format!("field '{field_name}': duplicate range constraint"))
            }
            (Length { .. }, Length { .. }) => {
                Some(format!("field '{field_name}': duplicate length constraint"))
            }
            (Pattern(_), Pattern(_)) => Some(format!(
                "field '{field_name}': duplicate pattern constraint"
            )),
            // Categories is closed-set; any other narrowing is redundant or
            // contradictory.
            (Categories(_), _) | (_, Categories(_)) => Some(format!(
                "field '{field_name}': categories cannot be combined with other constraints"
            )),
            // Range applies to numeric fields, Length/Pattern to string fields.
            // The type checker prevents both applying to the same field, so
            // these pairings shouldn't reach here in practice — but if they
            // do, flag explicitly.
            (Range { .. }, Length { .. }) | (Length { .. }, Range { .. }) => Some(format!(
                "field '{field_name}': range and length constraints apply to different field types"
            )),
            (Range { .. }, Pattern(_)) | (Pattern(_), Range { .. }) => Some(format!(
                "field '{field_name}': range and pattern constraints apply to different field types"
            )),
            // Length + Pattern: complementary string constraints, no conflict.
            (Length { .. }, Pattern(_)) | (Pattern(_), Length { .. }) => None,
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

    #[test]
    fn conflicts_with_duplicate_length() {
        let a = ConstraintKind::Length {
            min: Some(1),
            max: None,
        };
        let b = ConstraintKind::Length {
            min: None,
            max: Some(10),
        };
        let err = a.conflicts_with(&b, "f", &FieldType::String).unwrap();
        assert!(err.contains("duplicate length"));
    }

    #[test]
    fn conflicts_with_duplicate_pattern() {
        let a = ConstraintKind::Pattern("^a$".into());
        let b = ConstraintKind::Pattern("^b$".into());
        let err = a.conflicts_with(&b, "f", &FieldType::String).unwrap();
        assert!(err.contains("duplicate pattern"));
    }

    #[test]
    fn conflicts_with_categories_and_length() {
        let a = cats_kind(str_cats(&["a"]));
        let b = ConstraintKind::Length {
            min: Some(1),
            max: None,
        };
        let err = a.conflicts_with(&b, "f", &FieldType::String).unwrap();
        assert!(err.contains("categories cannot be combined"));
    }

    #[test]
    fn conflicts_with_categories_and_pattern() {
        let a = cats_kind(str_cats(&["a"]));
        let b = ConstraintKind::Pattern("^a$".into());
        let err = a.conflicts_with(&b, "f", &FieldType::String).unwrap();
        assert!(err.contains("categories cannot be combined"));
    }

    #[test]
    fn length_and_pattern_compatible() {
        let a = ConstraintKind::Length {
            min: Some(1),
            max: Some(10),
        };
        let b = ConstraintKind::Pattern("^[A-Z]+$".into());
        assert!(a.conflicts_with(&b, "f", &FieldType::String).is_none());
        assert!(b.conflicts_with(&a, "f", &FieldType::String).is_none());
    }

    #[test]
    fn validate_config_rejects_categories_with_length() {
        let c = Constraints {
            categories: Some(str_cats(&["a", "b"])),
            min_length: Some(1),
            ..Default::default()
        };
        let errors = c.validate_config("f", &FieldType::String);
        assert!(
            errors.iter().any(|e| e.contains("cannot be combined")),
            "expected cannot-be-combined error, got: {errors:?}"
        );
    }

    #[test]
    fn validate_config_accepts_length_plus_pattern() {
        let c = Constraints {
            min_length: Some(3),
            max_length: Some(64),
            pattern: Some("^[A-Z]".into()),
            ..Default::default()
        };
        assert!(c.validate_config("f", &FieldType::String).is_empty());
    }

    #[test]
    fn validate_config_rejects_length_on_integer() {
        let c = Constraints {
            min_length: Some(3),
            ..Default::default()
        };
        let errors = c.validate_config("f", &FieldType::Integer);
        assert!(!errors.is_empty());
    }

    #[test]
    fn validate_config_rejects_invalid_regex() {
        let c = Constraints {
            pattern: Some("[unclosed".into()),
            ..Default::default()
        };
        let errors = c.validate_config("f", &FieldType::String);
        assert!(errors.iter().any(|e| e.contains("not a valid regex")));
    }

    #[test]
    fn active_with_length_and_pattern() {
        let c = Constraints {
            min_length: Some(1),
            max_length: Some(10),
            pattern: Some("^a$".into()),
            ..Default::default()
        };
        let active = c.active();
        assert_eq!(active.len(), 2);
        assert!(matches!(active[0], ConstraintKind::Length { .. }));
        assert!(matches!(active[1], ConstraintKind::Pattern(_)));
    }
}
