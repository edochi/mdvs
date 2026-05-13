//! Outcome types for the validate leaf step.

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::{FieldViolation, NewField, format_file_count};

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
