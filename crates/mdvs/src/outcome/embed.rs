//! Outcome types for the embed leaf steps (EmbedFiles, EmbedQuery).

use serde::Serialize;

use crate::block::{Block, Render};

/// Full outcome for the embed_files step.
#[derive(Debug, Serialize)]
pub struct EmbedFilesOutcome {
    /// Number of files embedded.
    pub files_embedded: usize,
    /// Number of chunks produced.
    pub chunks_produced: usize,
}

impl Render for EmbedFilesOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!(
            "Embed: {} files, {} chunks",
            self.files_embedded, self.chunks_produced
        ))]
    }
}

/// Full outcome for the embed_query step.
#[derive(Debug, Serialize)]
pub struct EmbedQueryOutcome {
    /// The query string that was embedded.
    pub query: String,
}

impl Render for EmbedQueryOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!("Embed query: \"{}\"", self.query))]
    }
}
