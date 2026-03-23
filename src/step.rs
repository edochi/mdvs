//! Flat command result types for structured output.
//!
//! Every command returns a `CommandResult` with a flat list of process steps
//! and a final result. No recursive nesting — steps are always leaf entries.

use crate::block::{Block, Render};
use crate::outcome::Outcome;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::time::Instant;

/// A process step that completed successfully.
#[derive(Debug)]
pub struct ProcessStep {
    /// The step's typed outcome data.
    pub outcome: Outcome,
    /// Wall-clock time for this step in milliseconds.
    pub elapsed_ms: u64,
}

/// A process step that failed.
#[derive(Debug)]
pub struct FailedStep {
    /// Whether this is a user error or application error.
    pub kind: ErrorKind,
    /// Human-readable error message.
    pub message: String,
    /// Wall-clock time before failure in milliseconds.
    pub elapsed_ms: u64,
}

/// An entry in the command's step list.
#[derive(Debug)]
pub enum StepEntry {
    /// The step ran successfully.
    Completed(ProcessStep),
    /// The step failed.
    Failed(FailedStep),
    /// The step was skipped (not needed based on command logic).
    Skipped,
}

impl StepEntry {
    /// Create a successful step entry.
    pub fn ok(outcome: Outcome, elapsed_ms: u64) -> Self {
        Self::Completed(ProcessStep {
            outcome,
            elapsed_ms,
        })
    }

    /// Create a failed step entry.
    pub fn err(kind: ErrorKind, message: String, elapsed_ms: u64) -> Self {
        Self::Failed(FailedStep {
            kind,
            message,
            elapsed_ms,
        })
    }

    /// Create a skipped step entry.
    pub fn skipped() -> Self {
        Self::Skipped
    }
}

/// An error that occurred during a step or command.
#[derive(Debug, Clone, Serialize)]
pub struct StepError {
    /// Whether this is a user error (bad input) or application error (internal failure).
    pub kind: ErrorKind,
    /// Human-readable error message.
    pub message: String,
}

/// Error categorization (HTTP analogy: User ≈ 4xx, Application ≈ 5xx).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Bad input: config not found, model mismatch, invalid flags.
    User,
    /// Unexpected internal failure: I/O errors, parquet corruption.
    Application,
}

/// Result of running a command.
///
/// Contains a flat list of process steps and a final result (success or error).
#[derive(Debug)]
pub struct CommandResult {
    /// Process steps that ran (or were skipped) during this command.
    pub steps: Vec<StepEntry>,
    /// The command's final result: `Ok` with outcome data, or `Err` with error details.
    pub result: Result<Outcome, StepError>,
    /// Total wall-clock time for the entire command in milliseconds.
    pub elapsed_ms: u64,
}

impl CommandResult {
    /// Returns a reference to the successful outcome value, if any.
    pub fn result_value(&self) -> Option<&Outcome> {
        self.result.as_ref().ok()
    }

    /// Render verbose output: process step lines + command outcome.
    pub fn render_verbose(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Render each step with timing
        for entry in &self.steps {
            blocks.extend(entry.render());
        }

        // Render command outcome
        match &self.result {
            Ok(outcome) => blocks.extend(outcome.render()),
            Err(e) => blocks.push(Block::Line(format!("Error: {}", e.message))),
        }

        blocks
    }

    /// Create a failed result by extracting the error message from the last failed step.
    pub fn failed_from_steps(steps: Vec<StepEntry>, start: Instant) -> Self {
        let msg = steps
            .iter()
            .rev()
            .find_map(|s| match s {
                StepEntry::Failed(f) => Some(f.message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "step failed".into());
        Self {
            steps,
            result: Err(StepError {
                kind: ErrorKind::Application,
                message: msg,
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Create a failed result with an explicit error.
    pub fn failed(steps: Vec<StepEntry>, kind: ErrorKind, message: String, start: Instant) -> Self {
        Self {
            steps,
            result: Err(StepError { kind, message }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        }
    }

    /// Render compact output: command outcome only.
    pub fn render_compact(&self) -> Vec<Block> {
        match &self.result {
            Ok(outcome) => outcome.render(),
            Err(e) => vec![Block::Line(format!("Error: {}", e.message))],
        }
    }
}

// --- Free functions ---

/// Returns `true` if the command or any step failed.
pub fn has_failed(result: &CommandResult) -> bool {
    result.result.is_err()
        || result
            .steps
            .iter()
            .any(|s| matches!(s, StepEntry::Failed(_)))
}

/// Returns `true` if any outcome contains validation violations.
pub fn has_violations(result: &CommandResult) -> bool {
    let command_violations = match &result.result {
        Ok(outcome) => outcome.contains_violations(),
        Err(_) => false,
    };
    command_violations
        || result.steps.iter().any(|s| match s {
            StepEntry::Completed(ps) => ps.outcome.contains_violations(),
            _ => false,
        })
}

// --- Render impl for StepEntry ---

impl Render for StepEntry {
    fn render(&self) -> Vec<Block> {
        match self {
            Self::Completed(ps) => {
                let outcome_blocks = ps.outcome.render();
                // Inject timing into the first Block::Line
                let mut result = vec![];
                let mut injected = false;
                for block in outcome_blocks {
                    if !injected {
                        if let Block::Line(text) = block {
                            result.push(Block::Line(format!("{text} ({}ms)", ps.elapsed_ms)));
                            injected = true;
                            continue;
                        }
                    }
                    result.push(block);
                }
                result
            }
            Self::Failed(fs) => {
                vec![Block::Line(format!(
                    "Error: {} ({}ms)",
                    fs.message, fs.elapsed_ms
                ))]
            }
            Self::Skipped => vec![],
        }
    }
}

// --- Serialize impls ---

impl Serialize for CommandResult {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        map.serialize_entry("steps", &self.steps)?;
        match &self.result {
            Ok(outcome) => map.serialize_entry("result", outcome)?,
            Err(error) => map.serialize_entry("error", error)?,
        }
        map.serialize_entry("elapsed_ms", &self.elapsed_ms)?;
        map.end()
    }
}

impl Serialize for StepEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Completed(ps) => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("status", "complete")?;
                map.serialize_entry("elapsed_ms", &ps.elapsed_ms)?;
                map.serialize_entry("outcome", &ps.outcome)?;
                map.end()
            }
            Self::Failed(fs) => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("status", "failed")?;
                map.serialize_entry("elapsed_ms", &fs.elapsed_ms)?;
                map.serialize_entry(
                    "error",
                    &StepError {
                        kind: fs.kind.clone(),
                        message: fs.message.clone(),
                    },
                )?;
                map.end()
            }
            Self::Skipped => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("status", "skipped")?;
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::{CleanOutcome, DeleteIndexOutcome, Outcome};
    use std::path::PathBuf;

    #[test]
    fn step_entry_ok() {
        let entry = StepEntry::ok(
            Outcome::Scan(crate::outcome::ScanOutcome {
                files_found: 5,
                glob: "**".into(),
            }),
            42,
        );
        assert!(matches!(entry, StepEntry::Completed(_)));
    }

    #[test]
    fn step_entry_err() {
        let entry = StepEntry::err(ErrorKind::User, "not found".into(), 2);
        assert!(matches!(entry, StepEntry::Failed(_)));
    }

    #[test]
    fn step_entry_skipped() {
        let entry = StepEntry::skipped();
        assert!(matches!(entry, StepEntry::Skipped));
    }

    #[test]
    fn command_result_has_failed_on_err() {
        let result = CommandResult {
            steps: vec![],
            result: Err(StepError {
                kind: ErrorKind::User,
                message: "config not found".into(),
            }),
            elapsed_ms: 2,
        };
        assert!(has_failed(&result));
    }

    #[test]
    fn command_result_has_failed_on_step_failure() {
        let result = CommandResult {
            steps: vec![StepEntry::err(
                ErrorKind::Application,
                "scan failed".into(),
                0,
            )],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 1,
                size_bytes: 100,
            })),
            elapsed_ms: 5,
        };
        assert!(has_failed(&result));
    }

    #[test]
    fn command_result_success() {
        let result = CommandResult {
            steps: vec![StepEntry::ok(
                Outcome::DeleteIndex(DeleteIndexOutcome {
                    removed: true,
                    path: ".mdvs".into(),
                    files_removed: 2,
                    size_bytes: 1024,
                }),
                3,
            )],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 2,
                size_bytes: 1024,
            })),
            elapsed_ms: 5,
        };
        assert!(!has_failed(&result));
        assert!(result.result_value().is_some());
    }

    #[test]
    fn render_step_entry_with_timing() {
        let entry = StepEntry::ok(
            Outcome::Scan(crate::outcome::ScanOutcome {
                files_found: 43,
                glob: "**".into(),
            }),
            15,
        );
        let blocks = entry.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Scan: 43 files (15ms)"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_failed_step() {
        let entry = StepEntry::err(ErrorKind::User, "config not found".into(), 2);
        let blocks = entry.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Error: config not found (2ms)"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_skipped_step_empty() {
        let entry = StepEntry::skipped();
        assert!(entry.render().is_empty());
    }

    #[test]
    fn render_verbose() {
        let result = CommandResult {
            steps: vec![StepEntry::ok(
                Outcome::Scan(crate::outcome::ScanOutcome {
                    files_found: 5,
                    glob: "**".into(),
                }),
                10,
            )],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 1,
                size_bytes: 100,
            })),
            elapsed_ms: 15,
        };
        let blocks = result.render_verbose();
        assert_eq!(blocks.len(), 3); // step line + 2 clean lines
        match &blocks[0] {
            Block::Line(s) => assert!(s.contains("Scan") && s.contains("(10ms)")),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_compact_no_steps() {
        let result = CommandResult {
            steps: vec![StepEntry::ok(
                Outcome::Scan(crate::outcome::ScanOutcome {
                    files_found: 5,
                    glob: "**".into(),
                }),
                10,
            )],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 1,
                size_bytes: 100,
            })),
            elapsed_ms: 15,
        };
        let blocks = result.render_compact();
        assert_eq!(blocks.len(), 2); // 2 clean lines, no step line
    }

    #[test]
    fn serialize_verbose_json() {
        let result = CommandResult {
            steps: vec![StepEntry::ok(
                Outcome::DeleteIndex(DeleteIndexOutcome {
                    removed: true,
                    path: ".mdvs".into(),
                    files_removed: 1,
                    size_bytes: 512,
                }),
                3,
            )],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 1,
                size_bytes: 512,
            })),
            elapsed_ms: 5,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["steps"].as_array().unwrap().len(), 1);
        assert_eq!(json["steps"][0]["status"], "complete");
        assert!(json["steps"][0]["outcome"]["removed"].is_boolean()); // untagged: fields directly
        assert!(json["result"]["removed"].is_boolean()); // untagged: no "Clean" wrapper
        assert_eq!(json["elapsed_ms"], 5);
        assert!(json.get("error").is_none());
    }

    #[test]
    fn serialize_error_json() {
        let result = CommandResult {
            steps: vec![StepEntry::err(
                ErrorKind::User,
                "config not found".into(),
                2,
            )],
            result: Err(StepError {
                kind: ErrorKind::User,
                message: "config not found".into(),
            }),
            elapsed_ms: 2,
        };
        let json = serde_json::to_value(&result).unwrap();
        assert!(json.get("result").is_none());
        assert_eq!(json["error"]["kind"], "user");
        assert_eq!(json["error"]["message"], "config not found");
        assert_eq!(json["steps"][0]["status"], "failed");
    }

    #[test]
    fn step_error_is_clone() {
        let err = StepError {
            kind: ErrorKind::Application,
            message: "I/O error".into(),
        };
        let cloned = err.clone();
        assert_eq!(cloned.message, "I/O error");
    }

    // --- has_violations tests ---

    #[test]
    fn has_violations_none() {
        let result = CommandResult {
            steps: vec![],
            result: Ok(Outcome::Clean(CleanOutcome {
                removed: true,
                path: PathBuf::from(".mdvs"),
                files_removed: 1,
                size_bytes: 100,
            })),
            elapsed_ms: 5,
        };
        assert!(!has_violations(&result));
    }

    #[test]
    fn has_violations_in_result() {
        use crate::outcome::CheckOutcome;
        use crate::output::{FieldViolation, ViolatingFile, ViolationKind};

        let result = CommandResult {
            steps: vec![],
            result: Ok(Outcome::Check(Box::new(CheckOutcome {
                files_checked: 1,
                violations: vec![FieldViolation {
                    field: "draft".into(),
                    kind: ViolationKind::WrongType,
                    rule: "type Boolean".into(),
                    files: vec![ViolatingFile {
                        path: "post.md".into(),
                        detail: None,
                    }],
                }],
                new_fields: vec![],
            }))),
            elapsed_ms: 5,
        };
        assert!(has_violations(&result));
    }

    #[test]
    fn has_violations_in_step() {
        use crate::outcome::ValidateOutcome;
        use crate::output::{FieldViolation, ViolatingFile, ViolationKind};

        let result = CommandResult {
            steps: vec![StepEntry::ok(
                Outcome::Validate(ValidateOutcome {
                    files_checked: 1,
                    violations: vec![FieldViolation {
                        field: "title".into(),
                        kind: ViolationKind::MissingRequired,
                        rule: "required".into(),
                        files: vec![ViolatingFile {
                            path: "bare.md".into(),
                            detail: None,
                        }],
                    }],
                    new_fields: vec![],
                }),
                10,
            )],
            result: Err(StepError {
                kind: ErrorKind::User,
                message: "violations found".into(),
            }),
            elapsed_ms: 15,
        };
        assert!(has_violations(&result));
    }
}
