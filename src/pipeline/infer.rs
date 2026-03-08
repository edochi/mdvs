//! Infer step — infers field types and glob patterns from scanned files.

use serde::Serialize;
use std::time::Instant;

use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::pipeline::{ProcessingStep, ProcessingStepResult, StepOutput};

/// Output record for the infer step.
#[derive(Debug, Serialize)]
pub struct InferOutput {
    /// Number of fields inferred from frontmatter.
    pub fields_inferred: usize,
}

impl StepOutput for InferOutput {
    fn format_line(&self) -> String {
        if self.fields_inferred == 0 {
            "Inferred schema — no fields found".to_string()
        } else {
            format!("Inferred {} field(s)", self.fields_inferred)
        }
    }
}

/// Infer field types and glob patterns from scanned files.
///
/// Inference is infallible (pure computation), so always returns Completed.
/// Returns the step result and the inferred schema for subsequent steps.
pub fn run_infer(
    scanned: &ScannedFiles,
) -> (ProcessingStepResult<InferOutput>, Option<InferredSchema>) {
    let start = Instant::now();
    let schema = InferredSchema::infer(scanned);
    let step = ProcessingStep {
        elapsed_ms: start.elapsed().as_millis() as u64,
        output: InferOutput {
            fields_inferred: schema.fields.len(),
        },
    };
    (ProcessingStepResult::Completed(step), Some(schema))
}
