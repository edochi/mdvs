//! Outcome types for the infer leaf step.

use serde::Serialize;

use crate::block::{Block, Render};

/// Full outcome for the infer step.
#[derive(Debug, Serialize)]
pub struct InferOutcome {
    /// Number of fields inferred from frontmatter.
    pub fields_inferred: usize,
}

impl Render for InferOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!(
            "Infer: {} field(s)",
            self.fields_inferred
        ))]
    }
}
