//! Outcome types for search-related leaf steps (ExecuteSearch).

use serde::Serialize;

use crate::block::{Block, Render};

/// Full outcome for the execute_search step.
#[derive(Debug, Serialize)]
pub struct ExecuteSearchOutcome {
    /// Number of hits found.
    pub hits: usize,
}

impl Render for ExecuteSearchOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!("Execute search: {} hits", self.hits))]
    }
}
