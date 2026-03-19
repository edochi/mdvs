//! Core Step tree types for structured command output.
//!
//! Every command returns a `Step<O>` tree where `O` is the outcome type.
//! Leaf steps (scan, validate, etc.) have empty substeps. Commands (build,
//! search, etc.) have populated substeps forming a tree that mirrors the
//! execution pipeline.
//!
//! Two instantiations: `Step<Outcome>` (full data, verbose) and
//! `Step<CompactOutcome>` (summary data, compact). Conversion between
//! them is recursive via `to_compact()`.

use crate::block::{Block, Render};
use crate::outcome::{CompactOutcome, Outcome};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};

/// A Step tree with full outcome data (verbose mode).
pub type FullStep = Step<Outcome>;

/// A Step tree with compact outcome data (compact mode).
pub type CompactStep = Step<CompactOutcome>;

/// A node in the execution tree.
///
/// Leaf steps have `substeps: vec![]`. Commands have populated substeps
/// representing the pipeline stages that ran.
#[derive(Debug)]
pub struct Step<O> {
    /// Child steps that ran as part of this step's pipeline.
    pub substeps: Vec<Step<O>>,
    /// The outcome of this step itself.
    pub outcome: StepOutcome<O>,
}

/// The result of executing a step.
#[derive(Debug)]
pub enum StepOutcome<O> {
    /// The step ran, producing a successful outcome or an error.
    Complete {
        /// The step's result: `Ok` with outcome data, or `Err` with error details.
        result: Result<O, StepError>,
        /// Wall-clock time for this step in milliseconds.
        elapsed_ms: u64,
    },
    /// The step was skipped (upstream failure, not needed, !verbose, etc.).
    Skipped,
}

impl<O> StepOutcome<O> {
    /// Returns the elapsed time if the step completed, `None` if skipped.
    pub fn elapsed_ms(&self) -> Option<u64> {
        match self {
            Self::Complete { elapsed_ms, .. } => Some(*elapsed_ms),
            Self::Skipped => None,
        }
    }
}

impl Step<Outcome> {
    /// Recursively convert the full tree to a compact tree.
    ///
    /// Each outcome is converted via `Outcome::to_compact()`, which may
    /// read substep data for command-level summaries. Errors and Skipped
    /// outcomes are preserved as-is.
    pub fn to_compact(&self) -> Step<CompactOutcome> {
        let compact_outcome = match &self.outcome {
            StepOutcome::Complete {
                result: Ok(outcome),
                elapsed_ms,
            } => StepOutcome::Complete {
                result: Ok(outcome.to_compact(&self.substeps)),
                elapsed_ms: *elapsed_ms,
            },
            StepOutcome::Complete {
                result: Err(e),
                elapsed_ms,
            } => StepOutcome::Complete {
                result: Err(e.clone()),
                elapsed_ms: *elapsed_ms,
            },
            StepOutcome::Skipped => StepOutcome::Skipped,
        };
        Step {
            substeps: self.substeps.iter().map(|s| s.to_compact()).collect(),
            outcome: compact_outcome,
        }
    }
}

/// An error that occurred during a step.
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

impl<O> Step<O> {
    /// Create a leaf step (no substeps) with a successful outcome.
    pub fn leaf(outcome: O, elapsed_ms: u64) -> Self {
        Self {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(outcome),
                elapsed_ms,
            },
        }
    }

    /// Create a leaf step with a failed outcome.
    pub fn failed(kind: ErrorKind, message: String, elapsed_ms: u64) -> Self {
        Self {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError { kind, message }),
                elapsed_ms,
            },
        }
    }

    /// Create a skipped step.
    pub fn skipped() -> Self {
        Self {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        }
    }

    /// Compatibility method during migration — delegates to `has_failed()` free function.
    /// TODO: remove after all commands are converted (TODO-0131 wave 7).
    pub fn has_failed_step(&self) -> bool {
        has_failed(self)
    }
}

// --- Free functions ---

/// Returns `true` if any step in the tree failed (has `Err` outcome).
pub fn has_failed<O>(step: &Step<O>) -> bool {
    step.substeps.iter().any(|s| has_failed(s))
        || matches!(step.outcome, StepOutcome::Complete { result: Err(_), .. })
}

/// Returns `true` if any step in the tree contains validation violations.
pub fn has_violations(step: &Step<Outcome>) -> bool {
    step.substeps.iter().any(has_violations)
        || match &step.outcome {
            StepOutcome::Complete {
                result: Ok(outcome),
                ..
            } => outcome.contains_violations(),
            _ => false,
        }
}

// --- Pipeline migration helper (temporary, deleted in TODO-0131) ---

/// Convert an old `ProcessingStepResult<T>` into a new `Step<Outcome>`.
///
/// This is migration glue: during the transition, commands still call old
/// `run_*()` pipeline functions that return `ProcessingStepResult<T>`. This
/// generic helper converts the result into a `Step<Outcome>` leaf node,
/// using the provided closure to map the output to an `Outcome` variant.
///
/// Deleted when the old pipeline modules are removed (TODO-0131).
pub fn from_pipeline_result<T: Serialize, F>(
    result: crate::pipeline::ProcessingStepResult<T>,
    to_outcome: F,
) -> Step<Outcome>
where
    F: FnOnce(&T) -> Outcome,
{
    match result {
        crate::pipeline::ProcessingStepResult::Completed(step) => Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(to_outcome(&step.output)),
                elapsed_ms: step.elapsed_ms,
            },
        },
        crate::pipeline::ProcessingStepResult::Failed(err) => Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: convert_error_kind(err.kind),
                    message: err.message,
                }),
                elapsed_ms: 0,
            },
        },
        crate::pipeline::ProcessingStepResult::Skipped => Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        },
    }
}

/// Like [`from_pipeline_result`], but also returns the raw output data
/// (needed when subsequent steps consume data from a prior step).
pub fn from_pipeline_result_with_data<T: Serialize, F>(
    result: crate::pipeline::ProcessingStepResult<T>,
    to_outcome: F,
) -> (Step<Outcome>, Option<T>)
where
    F: FnOnce(&T) -> Outcome,
{
    match result {
        crate::pipeline::ProcessingStepResult::Completed(step) => {
            let outcome = to_outcome(&step.output);
            (
                Step {
                    substeps: vec![],
                    outcome: StepOutcome::Complete {
                        result: Ok(outcome),
                        elapsed_ms: step.elapsed_ms,
                    },
                },
                Some(step.output),
            )
        }
        crate::pipeline::ProcessingStepResult::Failed(err) => (
            Step {
                substeps: vec![],
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: convert_error_kind(err.kind),
                        message: err.message,
                    }),
                    elapsed_ms: 0,
                },
            },
            None,
        ),
        crate::pipeline::ProcessingStepResult::Skipped => (
            Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            },
            None,
        ),
    }
}

/// Convert old pipeline ErrorKind to new step ErrorKind.
/// Temporary migration helper (deleted in TODO-0131).
pub fn convert_error_kind(kind: crate::pipeline::ErrorKind) -> ErrorKind {
    match kind {
        crate::pipeline::ErrorKind::User => ErrorKind::User,
        crate::pipeline::ErrorKind::Application => ErrorKind::Application,
    }
}

// --- Serialize impls (hand-written, not derived) ---

impl<O: Serialize> Serialize for Step<O> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("substeps", &self.substeps)?;
        map.serialize_entry("outcome", &self.outcome)?;
        map.end()
    }
}

impl<O: Serialize> Serialize for StepOutcome<O> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Complete {
                result: Ok(outcome),
                elapsed_ms,
            } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("status", "complete")?;
                map.serialize_entry("elapsed_ms", elapsed_ms)?;
                map.serialize_entry("outcome", outcome)?;
                map.end()
            }
            Self::Complete {
                result: Err(error),
                elapsed_ms,
            } => {
                let mut map = serializer.serialize_map(Some(3))?;
                map.serialize_entry("status", "failed")?;
                map.serialize_entry("elapsed_ms", elapsed_ms)?;
                map.serialize_entry("error", error)?;
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

// --- Render impls ---

impl<O: Render> Render for Step<O> {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Render all substeps first
        for substep in &self.substeps {
            blocks.extend(substep.render());
        }

        // Render own outcome — with timing injection for leaf steps
        let outcome_blocks = self.outcome.render();
        if self.substeps.is_empty() {
            // Leaf step: inject elapsed_ms into the first Block::Line
            if let Some(elapsed) = self.outcome.elapsed_ms() {
                let mut injected = false;
                for block in outcome_blocks {
                    if !injected {
                        if let Block::Line(text) = block {
                            blocks.push(Block::Line(format!("{text} ({elapsed}ms)")));
                            injected = true;
                            continue;
                        }
                    }
                    blocks.push(block);
                }
            } else {
                blocks.extend(outcome_blocks);
            }
        } else {
            // Command step: no timing injection, outcome renders as-is
            blocks.extend(outcome_blocks);
        }

        blocks
    }
}

impl<O: Render> Render for StepOutcome<O> {
    fn render(&self) -> Vec<Block> {
        match self {
            Self::Complete {
                result: Ok(outcome),
                ..
            } => outcome.render(),
            Self::Complete { result: Err(e), .. } => {
                vec![Block::Line(format!("Error: {}", e.message))]
            }
            Self::Skipped => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaf_step_complete() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok("scanned 5 files".to_string()),
                elapsed_ms: 42,
            },
        };
        assert_eq!(step.outcome.elapsed_ms(), Some(42));
        assert!(step.substeps.is_empty());
    }

    #[test]
    fn leaf_step_failed() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: "config not found".into(),
                }),
                elapsed_ms: 2,
            },
        };
        assert_eq!(step.outcome.elapsed_ms(), Some(2));
        match &step.outcome {
            StepOutcome::Complete { result: Err(e), .. } => {
                assert_eq!(e.message, "config not found");
            }
            _ => panic!("expected failed step"),
        }
    }

    #[test]
    fn skipped_step() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        };
        assert_eq!(step.outcome.elapsed_ms(), None);
    }

    #[test]
    fn step_tree_with_substeps() {
        let leaf1: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok("scan".into()),
                elapsed_ms: 10,
            },
        };
        let leaf2: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok("validate".into()),
                elapsed_ms: 5,
            },
        };
        let command: Step<String> = Step {
            substeps: vec![leaf1, leaf2],
            outcome: StepOutcome::Complete {
                result: Ok("build complete".into()),
                elapsed_ms: 15,
            },
        };
        assert_eq!(command.substeps.len(), 2);
        assert_eq!(command.outcome.elapsed_ms(), Some(15));
    }

    // --- Render tests ---

    /// Implement Render for String so we can test Step<String> rendering.
    impl Render for String {
        fn render(&self) -> Vec<Block> {
            vec![Block::Line(self.clone())]
        }
    }

    #[test]
    fn render_leaf_step_injects_timing() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok("Scan: 43 files".into()),
                elapsed_ms: 15,
            },
        };
        let blocks = step.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Scan: 43 files (15ms)"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_command_step_no_timing() {
        let leaf: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok("Scan: 5 files".into()),
                elapsed_ms: 10,
            },
        };
        let command: Step<String> = Step {
            substeps: vec![leaf],
            outcome: StepOutcome::Complete {
                result: Ok("Built index".into()),
                elapsed_ms: 100,
            },
        };
        let blocks = command.render();
        assert_eq!(blocks.len(), 2);
        // First block: substep with timing
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Scan: 5 files (10ms)"),
            _ => panic!("expected Line"),
        }
        // Second block: command outcome WITHOUT timing
        match &blocks[1] {
            Block::Line(s) => assert_eq!(s, "Built index"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_skipped_step_empty() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        };
        let blocks = step.render();
        assert!(blocks.is_empty());
    }

    #[test]
    fn render_error_step() {
        let step: Step<String> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: "config not found".into(),
                }),
                elapsed_ms: 2,
            },
        };
        let blocks = step.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Error: config not found (2ms)"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn render_empty_outcome_no_timing_crash() {
        // An outcome that renders to empty vec — timing injection does nothing
        struct EmptyOutcome;
        impl Render for EmptyOutcome {
            fn render(&self) -> Vec<Block> {
                vec![]
            }
        }
        let step: Step<EmptyOutcome> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(EmptyOutcome),
                elapsed_ms: 5,
            },
        };
        let blocks = step.render();
        assert!(blocks.is_empty());
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

    // --- Serialize tests ---

    #[test]
    fn serialize_step_complete_ok() {
        use crate::outcome::{CleanOutcome, Outcome};
        use std::path::PathBuf;

        let step = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(Outcome::Clean(CleanOutcome {
                    removed: true,
                    path: PathBuf::from(".mdvs"),
                    files_removed: 2,
                    size_bytes: 1024,
                })),
                elapsed_ms: 5,
            },
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["outcome"]["status"], "complete");
        assert_eq!(json["outcome"]["elapsed_ms"], 5);
        assert!(json["outcome"]["outcome"]["Clean"].is_object());
        assert_eq!(json["outcome"]["outcome"]["Clean"]["removed"], true);
        assert_eq!(json["substeps"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn serialize_step_complete_err() {
        let step: Step<Outcome> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: "config not found".into(),
                }),
                elapsed_ms: 2,
            },
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["outcome"]["status"], "failed");
        assert_eq!(json["outcome"]["elapsed_ms"], 2);
        assert_eq!(json["outcome"]["error"]["kind"], "user");
        assert_eq!(json["outcome"]["error"]["message"], "config not found");
    }

    #[test]
    fn serialize_step_skipped() {
        let step: Step<Outcome> = Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        };
        let json = serde_json::to_value(&step).unwrap();
        assert_eq!(json["outcome"]["status"], "skipped");
        assert!(json["outcome"].get("elapsed_ms").is_none());
    }

    #[test]
    fn serialize_step_tree_recursive() {
        use crate::outcome::{DeleteIndexOutcome, Outcome};

        let leaf = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(Outcome::DeleteIndex(DeleteIndexOutcome {
                    removed: true,
                    path: ".mdvs".into(),
                    files_removed: 1,
                    size_bytes: 512,
                })),
                elapsed_ms: 3,
            },
        };
        let command = Step {
            substeps: vec![leaf],
            outcome: StepOutcome::Complete {
                result: Ok(Outcome::Clean(crate::outcome::CleanOutcome {
                    removed: true,
                    path: std::path::PathBuf::from(".mdvs"),
                    files_removed: 1,
                    size_bytes: 512,
                })),
                elapsed_ms: 3,
            },
        };
        let json = serde_json::to_value(&command).unwrap();
        assert_eq!(json["substeps"].as_array().unwrap().len(), 1);
        let sub = &json["substeps"][0];
        assert_eq!(sub["outcome"]["status"], "complete");
        assert!(sub["outcome"]["outcome"]["DeleteIndex"].is_object());
    }

    #[test]
    fn serialize_compact_step() {
        use crate::outcome::{CleanOutcome, Outcome};
        use std::path::PathBuf;

        let step = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(Outcome::Clean(CleanOutcome {
                    removed: true,
                    path: PathBuf::from(".mdvs"),
                    files_removed: 2,
                    size_bytes: 1024,
                })),
                elapsed_ms: 5,
            },
        };
        let compact = step.to_compact();
        let json = serde_json::to_value(&compact).unwrap();
        assert_eq!(json["outcome"]["status"], "complete");
        assert!(json["outcome"]["outcome"]["Clean"].is_object());
        // Compact Clean has removed + path, no files_removed/size_bytes
        assert_eq!(json["outcome"]["outcome"]["Clean"]["removed"], true);
    }
}
