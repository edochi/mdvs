//! Outcome types for the load_model leaf step.

use serde::Serialize;

use crate::block::{Block, Render};

/// Full outcome for the load_model step.
#[derive(Debug, Serialize)]
pub struct LoadModelOutcome {
    /// Name of the embedding model loaded.
    pub model_name: String,
    /// Embedding dimension.
    pub dimension: usize,
}

impl Render for LoadModelOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!("Load model: {}", self.model_name))]
    }
}
