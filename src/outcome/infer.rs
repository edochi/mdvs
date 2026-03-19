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

/// Compact outcome for the infer step (identical — no verbose-only fields).
#[derive(Debug, Serialize)]
pub struct InferOutcomeCompact {
    /// Number of fields inferred from frontmatter.
    pub fields_inferred: usize,
}

impl Render for InferOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&InferOutcome> for InferOutcomeCompact {
    fn from(o: &InferOutcome) -> Self {
        Self {
            fields_inferred: o.fields_inferred,
        }
    }
}
