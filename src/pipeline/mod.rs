//! Core pipeline abstractions for structured command output.
//!
//! Every command is a flat sequence of processing steps, each producing a typed
//! result. These types are the building blocks that step modules and commands
//! compose.

use serde::Serialize;

/// Trait for step output types — provides a one-liner text description.
///
/// Each step output struct implements this to describe its result in a single
/// line, e.g. "Scanned 5 files" or "Loaded model minishlab/potion-base-8M".
pub trait StepOutput {
    /// One-liner description of the step's result.
    fn format_line(&self) -> String;
}

/// A record of a single pipeline step's execution.
#[derive(Debug, Serialize)]
pub struct ProcessingStep<T: Serialize> {
    /// Wall-clock time for this step in milliseconds.
    pub elapsed_ms: u64,
    /// Step-specific typed output.
    pub output: T,
}

/// The three states a step can be in.
///
/// Serialized with a `status` discriminator field (internally tagged):
/// - `{ "status": "completed", "elapsed_ms": 42, "output": { ... } }`
/// - `{ "status": "failed", "kind": "user", "message": "..." }`
/// - `{ "status": "skipped" }`
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ProcessingStepResult<T: Serialize> {
    /// The step ran and produced output.
    Completed(ProcessingStep<T>),
    /// The step ran but hit an actual error.
    Failed(ProcessingStepError),
    /// The step didn't run because a previous step's outcome made it unnecessary.
    Skipped,
}

impl<T: Serialize + StepOutput> ProcessingStepResult<T> {
    /// Render this step as a single line for text output.
    pub fn format_line(&self) -> String {
        match self {
            Self::Completed(step) => step.output.format_line(),
            Self::Failed(err) => format!("failed: {}", err.message),
            Self::Skipped => "skipped".to_string(),
        }
    }
}

/// An error that occurred during a processing step.
#[derive(Debug, Serialize)]
pub struct ProcessingStepError {
    /// Whether this is a user error (bad input) or application error (internal failure).
    pub kind: ErrorKind,
    /// Human-readable error message.
    pub message: String,
}

/// Error categorization (HTTP analogy).
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Bad input: config not found, model mismatch, invalid flags (4xx).
    User,
    /// Unexpected internal failure: I/O errors, parquet corruption (5xx).
    Application,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dummy step output for testing.
    #[derive(Debug, Serialize)]
    struct DummyOutput {
        files: usize,
    }

    impl StepOutput for DummyOutput {
        fn format_line(&self) -> String {
            format!("Processed {} files", self.files)
        }
    }

    #[test]
    fn completed_json_shape() {
        let result: ProcessingStepResult<DummyOutput> =
            ProcessingStepResult::Completed(ProcessingStep {
                elapsed_ms: 42,
                output: DummyOutput { files: 5 },
            });
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "completed");
        assert_eq!(json["elapsed_ms"], 42);
        assert_eq!(json["output"]["files"], 5);
    }

    #[test]
    fn failed_json_shape() {
        let result: ProcessingStepResult<DummyOutput> =
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: "config not found".into(),
            });
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["kind"], "user");
        assert_eq!(json["message"], "config not found");
    }

    #[test]
    fn failed_application_error_json_shape() {
        let result: ProcessingStepResult<DummyOutput> =
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::Application,
                message: "I/O error reading parquet".into(),
            });
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "failed");
        assert_eq!(json["kind"], "application");
        assert_eq!(json["message"], "I/O error reading parquet");
    }

    #[test]
    fn skipped_json_shape() {
        let result: ProcessingStepResult<DummyOutput> = ProcessingStepResult::Skipped;
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "skipped");
        // No other fields
        assert_eq!(json.as_object().unwrap().len(), 1);
    }

    #[test]
    fn format_line_completed() {
        let result: ProcessingStepResult<DummyOutput> =
            ProcessingStepResult::Completed(ProcessingStep {
                elapsed_ms: 100,
                output: DummyOutput { files: 3 },
            });
        assert_eq!(result.format_line(), "Processed 3 files");
    }

    #[test]
    fn format_line_failed() {
        let result: ProcessingStepResult<DummyOutput> =
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::User,
                message: "model not found".into(),
            });
        assert_eq!(result.format_line(), "failed: model not found");
    }

    #[test]
    fn format_line_skipped() {
        let result: ProcessingStepResult<DummyOutput> = ProcessingStepResult::Skipped;
        assert_eq!(result.format_line(), "skipped");
    }
}
