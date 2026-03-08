//! Load model step — loads the embedding model.

use serde::Serialize;
use std::time::Instant;

use crate::index::embed::{Embedder, ModelConfig};
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};
use crate::schema::shared::EmbeddingModelConfig;

/// Output record for the load model step.
#[derive(Debug, Serialize)]
pub struct LoadModelOutput {
    /// Name of the loaded model.
    pub model_name: String,
    /// Embedding dimension.
    pub dimension: usize,
}

impl StepOutput for LoadModelOutput {
    fn format_line(&self) -> String {
        format!("Loaded model \"{}\" ({}d)", self.model_name, self.dimension)
    }
}

/// Load the embedding model from config.
///
/// Returns the step result and the loaded embedder (for subsequent steps).
pub fn run_load_model(
    embedding: &EmbeddingModelConfig,
) -> (ProcessingStepResult<LoadModelOutput>, Option<Embedder>) {
    let start = Instant::now();

    let model_config = match ModelConfig::try_from(embedding) {
        Ok(mc) => mc,
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            return (ProcessingStepResult::Failed(err), None);
        }
    };

    match Embedder::load(&model_config) {
        Ok(embedder) => {
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: LoadModelOutput {
                    model_name: embedding.name.clone(),
                    dimension: embedder.dimension(),
                },
            };
            (ProcessingStepResult::Completed(step), Some(embedder))
        }
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
    }
}
