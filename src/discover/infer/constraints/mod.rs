//! Constraint inference — auto-detection of constraints from scanned data.
//!
//! Each constraint kind that supports auto-inference has its own submodule.
//! The [`infer_constraints`] orchestrator calls each and merges the results.

mod categories;

use super::InferredField;
use crate::schema::constraints::Constraints;

/// Run all constraint inference heuristics on an inferred field.
/// Returns `Some(Constraints)` if any constraints were detected, `None` otherwise.
pub fn infer_constraints(
    field: &InferredField,
    max_categories: usize,
    min_repetition: usize,
) -> Option<Constraints> {
    // Currently only categorical inference is auto-detected.
    // Future constraint kinds that support auto-inference would be called here
    // and their results merged into a single Constraints struct.
    categories::infer(field, max_categories, min_repetition)
}
