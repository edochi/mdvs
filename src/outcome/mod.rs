//! Outcome types for all pipeline steps and commands.
//!
//! The `Outcome` enum contains one variant per step/command.

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

pub use classify::ClassifyOutcome;
pub use commands::{
    BuildOutcome, CheckOutcome, CleanOutcome, InfoOutcome, InitOutcome, SearchOutcome,
    UpdateOutcome,
};
pub use config::{ReadConfigOutcome, WriteConfigOutcome};
pub use embed::{EmbedFilesOutcome, EmbedQueryOutcome};
pub use index::{DeleteIndexOutcome, ReadIndexOutcome, WriteIndexOutcome};
pub use infer::InferOutcome;
pub use model::LoadModelOutcome;
pub use scan::ScanOutcome;
pub use search::ExecuteSearchOutcome;
pub use validate::ValidateOutcome;

/// Outcome for all steps and commands.
///
/// Each variant wraps a named outcome struct carrying all data needed for
/// rendering and JSON serialization. Command-level outcomes are `Box`ed
/// to avoid bloating the enum.
#[derive(Debug, Serialize)]
#[serde(untagged)]
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
    /// Returns `true` if this outcome contains validation violations.
    pub fn contains_violations(&self) -> bool {
        match self {
            Self::Validate(v) => !v.violations.is_empty(),
            Self::Check(c) => !c.violations.is_empty(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn contains_violations_false_for_clean() {
        let outcome = Outcome::Clean(CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 1,
            size_bytes: 100,
        });
        assert!(!outcome.contains_violations());
    }
}
