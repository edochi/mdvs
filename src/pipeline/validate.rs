//! Validate step — checks frontmatter against the schema.

use serde::Serialize;
use std::time::Instant;

use crate::discover::scan::ScannedFiles;
use crate::output::{format_file_count, FieldViolation, NewField};
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};
use crate::schema::config::MdvsToml;

/// Output record for the validate step.
#[derive(Debug, Serialize)]
pub struct ValidateOutput {
    /// Number of markdown files validated.
    pub files_checked: usize,
    /// Number of schema violations found.
    pub violation_count: usize,
    /// Number of new (unknown) fields found.
    pub new_field_count: usize,
}

impl StepOutput for ValidateOutput {
    fn format_line(&self) -> String {
        if self.violation_count == 0 {
            format!("{} — no violations", format_file_count(self.files_checked))
        } else {
            format!(
                "{} — {} violation(s)",
                format_file_count(self.files_checked),
                self.violation_count
            )
        }
    }
}

/// Validate scanned files against the schema.
///
/// Full validation data passed forward to the command result.
pub type ValidationData = (Vec<FieldViolation>, Vec<NewField>);

/// Returns the step result (for the pipeline record) and the full validation
/// data (violations and new fields, for the command result). The validation
/// data is `None` if validation itself failed (not if violations were found —
/// violations are normal output).
pub fn run_validate(
    scanned: &ScannedFiles,
    config: &MdvsToml,
    verbose: bool,
) -> (ProcessingStepResult<ValidateOutput>, Option<ValidationData>) {
    let start = Instant::now();
    match crate::cmd::check::validate(scanned, config, verbose) {
        Ok(check_result) => {
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ValidateOutput {
                    files_checked: check_result.files_checked,
                    violation_count: check_result.field_violations.len(),
                    new_field_count: check_result.new_fields.len(),
                },
            };
            let data = (check_result.field_violations, check_result.new_fields);
            (ProcessingStepResult::Completed(step), Some(data))
        }
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
    }
}
