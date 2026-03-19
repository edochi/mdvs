//! Outcome types for all pipeline steps and commands.
//!
//! The `Outcome` and `CompactOutcome` enums contain one variant per step/command.
//! Variants are added incrementally as commands are converted to the Step tree
//! architecture.

pub mod classify;
pub mod commands;
pub mod config;
pub mod embed;
pub mod index;
pub mod infer;
pub mod model;
pub mod scan;
pub mod search;
pub mod validate;

use serde::Serialize;

use crate::block::{Block, Render};
use crate::step::Step;

pub use classify::{ClassifyOutcome, ClassifyOutcomeCompact};
pub use commands::{
    BuildOutcome, BuildOutcomeCompact, CheckOutcome, CheckOutcomeCompact, CleanOutcome,
    CleanOutcomeCompact, InfoOutcome, InfoOutcomeCompact, InitOutcome, InitOutcomeCompact,
    SearchOutcome, SearchOutcomeCompact, UpdateOutcome, UpdateOutcomeCompact,
};
pub use config::{
    ReadConfigOutcome, ReadConfigOutcomeCompact, WriteConfigOutcome, WriteConfigOutcomeCompact,
};
pub use embed::{
    EmbedFilesOutcome, EmbedFilesOutcomeCompact, EmbedQueryOutcome, EmbedQueryOutcomeCompact,
};
pub use index::{
    DeleteIndexOutcome, DeleteIndexOutcomeCompact, ReadIndexOutcome, ReadIndexOutcomeCompact,
    WriteIndexOutcome, WriteIndexOutcomeCompact,
};
pub use infer::{InferOutcome, InferOutcomeCompact};
pub use model::{LoadModelOutcome, LoadModelOutcomeCompact};
pub use scan::{ScanOutcome, ScanOutcomeCompact};
pub use search::{ExecuteSearchOutcome, ExecuteSearchOutcomeCompact};
pub use validate::{ValidateOutcome, ValidateOutcomeCompact};

/// Full outcome for all steps and commands.
///
/// Each variant wraps a named outcome struct carrying all data needed for
/// verbose rendering and JSON serialization. Command-level outcomes are
/// `Box`ed to avoid bloating the enum.
#[derive(Debug, Serialize)]
pub enum Outcome {
    /// Delete the `.mdvs/` directory.
    DeleteIndex(DeleteIndexOutcome),
    /// Read and parse `mdvs.toml`.
    ReadConfig(ReadConfigOutcome),
    /// Scan the project directory for markdown files.
    Scan(ScanOutcome),
    /// Read the existing index (parquet files).
    ReadIndex(ReadIndexOutcome),
    /// Validate frontmatter against the schema.
    Validate(ValidateOutcome),
    /// Clean command — delete `.mdvs/` and report.
    Clean(CleanOutcome),
    /// Check command — validate and report violations.
    Check(Box<CheckOutcome>),
    /// Infer field types and glob patterns.
    Infer(InferOutcome),
    /// Write `mdvs.toml` to disk.
    WriteConfig(WriteConfigOutcome),
    /// Info command — display config and index status.
    Info(Box<InfoOutcome>),
    /// Init command — scan, infer, write config.
    Init(Box<InitOutcome>),
    /// Classify files for incremental build.
    Classify(ClassifyOutcome),
    /// Load the embedding model.
    LoadModel(LoadModelOutcome),
    /// Embed files that need embedding.
    EmbedFiles(EmbedFilesOutcome),
    /// Write the index to disk.
    WriteIndex(WriteIndexOutcome),
    /// Update command — re-scan, re-infer, update config.
    Update(Box<UpdateOutcome>),
    /// Embed a query string.
    EmbedQuery(EmbedQueryOutcome),
    /// Execute search against the index.
    ExecuteSearch(ExecuteSearchOutcome),
    /// Build command — validate, embed, write index.
    Build(Box<BuildOutcome>),
    /// Search command — embed query, search index.
    Search(Box<SearchOutcome>),
}

impl Render for Outcome {
    fn render(&self) -> Vec<Block> {
        match self {
            Self::DeleteIndex(o) => o.render(),
            Self::ReadConfig(o) => o.render(),
            Self::Scan(o) => o.render(),
            Self::ReadIndex(o) => o.render(),
            Self::Validate(o) => o.render(),
            Self::Infer(o) => o.render(),
            Self::WriteConfig(o) => o.render(),
            Self::Clean(o) => o.render(),
            Self::Check(o) => o.render(),
            Self::Classify(o) => o.render(),
            Self::LoadModel(o) => o.render(),
            Self::EmbedFiles(o) => o.render(),
            Self::WriteIndex(o) => o.render(),
            Self::Info(o) => o.render(),
            Self::Init(o) => o.render(),
            Self::Update(o) => o.render(),
            Self::EmbedQuery(o) => o.render(),
            Self::ExecuteSearch(o) => o.render(),
            Self::Build(o) => o.render(),
            Self::Search(o) => o.render(),
        }
    }
}

impl Outcome {
    /// Convert this full outcome to its compact counterpart.
    ///
    /// Command outcomes may read `substeps` to derive summary data.
    /// Leaf outcomes ignore `substeps`.
    pub fn to_compact(&self, _substeps: &[Step<Outcome>]) -> CompactOutcome {
        match self {
            Self::DeleteIndex(o) => CompactOutcome::DeleteIndex(o.into()),
            Self::ReadConfig(o) => CompactOutcome::ReadConfig(o.into()),
            Self::Scan(o) => CompactOutcome::Scan(o.into()),
            Self::ReadIndex(o) => CompactOutcome::ReadIndex(o.into()),
            Self::Validate(o) => CompactOutcome::Validate(o.into()),
            Self::Infer(o) => CompactOutcome::Infer(o.into()),
            Self::WriteConfig(o) => CompactOutcome::WriteConfig(o.into()),
            Self::Clean(o) => CompactOutcome::Clean(o.into()),
            Self::Check(o) => CompactOutcome::Check(Box::new(o.as_ref().into())),
            Self::Info(o) => CompactOutcome::Info(Box::new(o.as_ref().into())),
            Self::Classify(o) => CompactOutcome::Classify(o.into()),
            Self::LoadModel(o) => CompactOutcome::LoadModel(o.into()),
            Self::EmbedFiles(o) => CompactOutcome::EmbedFiles(o.into()),
            Self::WriteIndex(o) => CompactOutcome::WriteIndex(o.into()),
            Self::Init(o) => CompactOutcome::Init(Box::new(o.as_ref().into())),
            Self::Update(o) => CompactOutcome::Update(Box::new(o.as_ref().into())),
            Self::EmbedQuery(o) => CompactOutcome::EmbedQuery(o.into()),
            Self::ExecuteSearch(o) => CompactOutcome::ExecuteSearch(o.into()),
            Self::Build(o) => CompactOutcome::Build(Box::new(o.as_ref().into())),
            Self::Search(o) => CompactOutcome::Search(Box::new(o.as_ref().into())),
        }
    }

    /// Returns `true` if this outcome contains validation violations.
    ///
    /// Used for exit code logic. Only Validate and Check outcomes can
    /// return `true` — added when those variants are implemented.
    pub fn contains_violations(&self) -> bool {
        match self {
            Self::Validate(v) => !v.violations.is_empty(),
            Self::Check(c) => !c.violations.is_empty(),
            Self::DeleteIndex(_)
            | Self::ReadConfig(_)
            | Self::Scan(_)
            | Self::ReadIndex(_)
            | Self::Infer(_)
            | Self::WriteConfig(_)
            | Self::Clean(_)
            | Self::Classify(_)
            | Self::LoadModel(_)
            | Self::EmbedFiles(_)
            | Self::WriteIndex(_)
            | Self::Info(_)
            | Self::Init(_)
            | Self::EmbedQuery(_)
            | Self::ExecuteSearch(_)
            | Self::Update(_)
            | Self::Build(_)
            | Self::Search(_) => false,
        }
    }
}

/// Compact outcome for all steps and commands.
///
/// Mirrors `Outcome` with compact counterpart structs. Leaf compact outcomes
/// render to empty vecs (silent). Command compact outcomes render summaries.
#[derive(Debug, Serialize)]
pub enum CompactOutcome {
    /// Delete the `.mdvs/` directory (compact).
    DeleteIndex(DeleteIndexOutcomeCompact),
    /// Read and parse `mdvs.toml` (compact).
    ReadConfig(ReadConfigOutcomeCompact),
    /// Scan the project directory (compact).
    Scan(ScanOutcomeCompact),
    /// Read the existing index (compact).
    ReadIndex(ReadIndexOutcomeCompact),
    /// Validate frontmatter (compact).
    Validate(ValidateOutcomeCompact),
    /// Clean command (compact).
    Clean(CleanOutcomeCompact),
    /// Check command (compact).
    Check(Box<CheckOutcomeCompact>),
    /// Infer (compact).
    Infer(InferOutcomeCompact),
    /// Write config (compact).
    WriteConfig(WriteConfigOutcomeCompact),
    /// Info command (compact).
    Info(Box<InfoOutcomeCompact>),
    /// Init command (compact).
    Init(Box<InitOutcomeCompact>),
    /// Classify (compact).
    Classify(ClassifyOutcomeCompact),
    /// Load model (compact).
    LoadModel(LoadModelOutcomeCompact),
    /// Embed files (compact).
    EmbedFiles(EmbedFilesOutcomeCompact),
    /// Write index (compact).
    WriteIndex(WriteIndexOutcomeCompact),
    /// Update command (compact).
    Update(Box<UpdateOutcomeCompact>),
    /// Embed query (compact).
    EmbedQuery(EmbedQueryOutcomeCompact),
    /// Execute search (compact).
    ExecuteSearch(ExecuteSearchOutcomeCompact),
    /// Build command (compact).
    Build(Box<BuildOutcomeCompact>),
    /// Search command (compact).
    Search(Box<SearchOutcomeCompact>),
}

impl Render for CompactOutcome {
    fn render(&self) -> Vec<Block> {
        match self {
            Self::DeleteIndex(o) => o.render(),
            Self::ReadConfig(o) => o.render(),
            Self::Scan(o) => o.render(),
            Self::ReadIndex(o) => o.render(),
            Self::Validate(o) => o.render(),
            Self::Infer(o) => o.render(),
            Self::WriteConfig(o) => o.render(),
            Self::Clean(o) => o.render(),
            Self::Check(o) => o.render(),
            Self::Classify(o) => o.render(),
            Self::LoadModel(o) => o.render(),
            Self::EmbedFiles(o) => o.render(),
            Self::WriteIndex(o) => o.render(),
            Self::Info(o) => o.render(),
            Self::Init(o) => o.render(),
            Self::Update(o) => o.render(),
            Self::EmbedQuery(o) => o.render(),
            Self::ExecuteSearch(o) => o.render(),
            Self::Build(o) => o.render(),
            Self::Search(o) => o.render(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::step::{ErrorKind, StepError, StepOutcome};
    use std::path::PathBuf;

    #[test]
    fn outcome_render_delegates() {
        let outcome = Outcome::Clean(CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 1,
            size_bytes: 100,
        });
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 2);
    }

    #[test]
    fn compact_outcome_render_delegates() {
        let outcome = CompactOutcome::Clean(CleanOutcomeCompact {
            removed: true,
            path: PathBuf::from(".mdvs"),
        });
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 1); // Compact command renders summary
    }

    #[test]
    fn compact_leaf_is_silent() {
        let outcome = CompactOutcome::DeleteIndex(DeleteIndexOutcomeCompact {
            removed: true,
            path: ".mdvs".into(),
            files_removed: 1,
            size_bytes: 100,
        });
        assert!(outcome.render().is_empty());
    }

    #[test]
    fn to_compact_roundtrip() {
        let outcome = Outcome::Clean(CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 3,
            size_bytes: 2048,
        });
        let compact = outcome.to_compact(&[]);
        match &compact {
            CompactOutcome::Clean(c) => {
                assert!(c.removed);
                assert_eq!(c.path, PathBuf::from(".mdvs"));
            }
            _ => panic!("expected Clean compact"),
        }
    }

    #[test]
    fn contains_violations_false_for_clean() {
        let outcome = Outcome::Clean(CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 1,
            size_bytes: 100,
        });
        assert!(!outcome.contains_violations());
    }

    #[test]
    fn step_to_compact_full_tree() {
        let leaf = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Ok(Outcome::DeleteIndex(DeleteIndexOutcome {
                    removed: true,
                    path: ".mdvs".into(),
                    files_removed: 2,
                    size_bytes: 1024,
                })),
                elapsed_ms: 5,
            },
        };
        let command = Step {
            substeps: vec![leaf],
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

        let compact = command.to_compact();
        assert_eq!(compact.substeps.len(), 1);
        // Leaf compact renders silent
        assert!(compact.substeps[0].render().is_empty());
        // Command compact renders summary
        let blocks = compact.render();
        assert!(!blocks.is_empty());
    }

    #[test]
    fn step_to_compact_error_preserved() {
        let step: Step<Outcome> = Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: "test error".into(),
                }),
                elapsed_ms: 1,
            },
        };
        let compact = step.to_compact();
        match &compact.outcome {
            StepOutcome::Complete { result: Err(e), .. } => assert_eq!(e.message, "test error"),
            _ => panic!("expected error preserved"),
        }
    }

    #[test]
    fn step_to_compact_skipped_preserved() {
        let step: Step<Outcome> = Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        };
        let compact = step.to_compact();
        assert!(matches!(compact.outcome, StepOutcome::Skipped));
    }
}
