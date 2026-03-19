//! Outcome types for the validate leaf step.

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::{
    format_file_count, FieldViolation, FieldViolationCompact, NewField, NewFieldCompact,
};

/// Full outcome for the validate step.
#[derive(Debug, Serialize)]
pub struct ValidateOutcome {
    /// Number of markdown files validated.
    pub files_checked: usize,
    /// Violations found during validation.
    pub violations: Vec<FieldViolation>,
    /// Fields found in frontmatter but not defined in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
}

impl Render for ValidateOutcome {
    fn render(&self) -> Vec<Block> {
        let violation_part = if self.violations.is_empty() {
            "no violations".to_string()
        } else {
            format!("{} violation(s)", self.violations.len())
        };
        vec![Block::Line(format!(
            "Validate: {} — {violation_part}",
            format_file_count(self.files_checked),
        ))]
    }
}

/// Compact outcome for the validate step.
#[derive(Debug, Serialize)]
pub struct ValidateOutcomeCompact {
    /// Number of markdown files validated.
    pub files_checked: usize,
    /// Compact violations (count only, no file paths).
    pub violations: Vec<FieldViolationCompact>,
    /// Compact new fields (count only, no file paths).
    pub new_fields: Vec<NewFieldCompact>,
}

impl Render for ValidateOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&ValidateOutcome> for ValidateOutcomeCompact {
    fn from(o: &ValidateOutcome) -> Self {
        Self {
            files_checked: o.files_checked,
            violations: o
                .violations
                .iter()
                .map(FieldViolationCompact::from)
                .collect(),
            new_fields: o.new_fields.iter().map(NewFieldCompact::from).collect(),
        }
    }
}
