//! Outcome types for the scan leaf step.

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::format_file_count;

/// Full outcome for the scan step.
#[derive(Debug, Serialize)]
pub struct ScanOutcome {
    /// Number of markdown files found.
    pub files_found: usize,
    /// Glob pattern used for scanning.
    pub glob: String,
}

impl Render for ScanOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!(
            "Scan: {}",
            format_file_count(self.files_found)
        ))]
    }
}

/// Compact outcome for the scan step (identical — no verbose-only fields).
#[derive(Debug, Serialize)]
pub struct ScanOutcomeCompact {
    /// Number of markdown files found.
    pub files_found: usize,
    /// Glob pattern used for scanning.
    pub glob: String,
}

impl Render for ScanOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&ScanOutcome> for ScanOutcomeCompact {
    fn from(o: &ScanOutcome) -> Self {
        Self {
            files_found: o.files_found,
            glob: o.glob.clone(),
        }
    }
}
