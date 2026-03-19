//! Outcome types for the embed leaf steps (EmbedFiles, EmbedQuery).
//!
//! Only EmbedFiles is defined initially. EmbedQuery added when search is converted.

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

/// Compact outcome for the embed_files step (identical).
#[derive(Debug, Serialize)]
pub struct EmbedFilesOutcomeCompact {
    /// Number of files embedded.
    pub files_embedded: usize,
    /// Number of chunks produced.
    pub chunks_produced: usize,
}

impl Render for EmbedFilesOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![]
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

/// Compact outcome for the embed_query step (identical).
#[derive(Debug, Serialize)]
pub struct EmbedQueryOutcomeCompact {
    /// The query string that was embedded.
    pub query: String,
}

impl Render for EmbedQueryOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![]
    }
}

impl From<&EmbedQueryOutcome> for EmbedQueryOutcomeCompact {
    fn from(o: &EmbedQueryOutcome) -> Self {
        Self {
            query: o.query.clone(),
        }
    }
}

impl From<&EmbedFilesOutcome> for EmbedFilesOutcomeCompact {
    fn from(o: &EmbedFilesOutcome) -> Self {
        Self {
            files_embedded: o.files_embedded,
            chunks_produced: o.chunks_produced,
        }
    }
}
