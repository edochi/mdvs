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
