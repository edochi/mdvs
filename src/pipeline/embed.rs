//! Embed step — embeds queries (and later, files) using the loaded model.

use serde::Serialize;
use std::time::Instant;

use crate::index::embed::Embedder;
use crate::pipeline::{ProcessingStep, ProcessingStepResult, StepOutput};

/// Output record for the embed query step.
#[derive(Debug, Serialize)]
pub struct EmbedQueryOutput {
    /// The query that was embedded.
    pub query: String,
}

impl StepOutput for EmbedQueryOutput {
    fn format_line(&self) -> String {
        format!("Embedded query \"{}\"", self.query)
    }
}

/// Embed a query string using the loaded model.
///
/// Returns the step result and the query embedding vector.
pub async fn run_embed_query(
    embedder: &Embedder,
    query: &str,
) -> (ProcessingStepResult<EmbedQueryOutput>, Option<Vec<f32>>) {
    let start = Instant::now();
    let embedding = embedder.embed(query).await;
    let step = ProcessingStep {
        elapsed_ms: start.elapsed().as_millis() as u64,
        output: EmbedQueryOutput {
            query: query.to_string(),
        },
    };
    (ProcessingStepResult::Completed(step), Some(embedding))
}
