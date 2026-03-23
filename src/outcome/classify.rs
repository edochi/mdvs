//! Outcome types for the classify leaf step.

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::format_file_count;

/// Full outcome for the classify step.
#[derive(Debug, Serialize)]
pub struct ClassifyOutcome {
    /// Whether this is a full rebuild.
    pub full_rebuild: bool,
    /// Number of files that need embedding (new + edited).
    pub needs_embedding: usize,
    /// Number of files unchanged from previous build.
    pub unchanged: usize,
    /// Number of files removed since previous build.
    pub removed: usize,
}

impl Render for ClassifyOutcome {
    fn render(&self) -> Vec<Block> {
        if self.full_rebuild {
            vec![Block::Line(format!(
                "Classify: {} (full rebuild)",
                format_file_count(self.needs_embedding)
            ))]
        } else {
            vec![Block::Line(format!(
                "Classify: {} to embed, {} unchanged, {} removed",
                self.needs_embedding, self.unchanged, self.removed
            ))]
        }
    }
}
