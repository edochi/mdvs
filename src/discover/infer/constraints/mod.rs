//! Constraint inference — auto-detection of constraints from scanned data.
//!
//! Each constraint kind that supports auto-inference has its own submodule.
//! The [`infer_constraints`] orchestrator calls each and merges the results.

mod categories;
pub(crate) mod range;

use super::InferredField;
use crate::schema::constraints::Constraints;

/// Run all constraint inference heuristics on an inferred field.
/// Returns `Some(Constraints)` if any constraints were detected, `None` otherwise.
///
/// Range constraints are never auto-inferred — they require explicit `--range`.
pub fn infer_constraints(
    field: &InferredField,
    max_categories: usize,
    min_repetition: usize,
) -> Option<Constraints> {
    categories::infer(field, max_categories, min_repetition)
}

/// Infer range (min/max) constraints from observed numeric values.
/// Returns `Some(Constraints)` with min and max set, `None` for non-numeric fields.
pub fn infer_range(field: &InferredField) -> Option<Constraints> {
    let (min, max) = range::infer(field)?;
    Some(Constraints {
        min: Some(min),
        max: Some(max),
        ..Default::default()
    })
}
