//! Read index step — reads backend metadata and statistics.

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

use crate::index::backend::{Backend, IndexStats};
use crate::index::storage::BuildMetadata;
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};

/// Output record for the read index step.
#[derive(Debug, Serialize)]
pub struct ReadIndexOutput {
    /// Whether a built index was found.
    pub exists: bool,
    /// Number of files in the index (0 if no index).
    pub files_indexed: usize,
    /// Number of chunks in the index (0 if no index).
    pub chunks: usize,
}

impl StepOutput for ReadIndexOutput {
    fn format_line(&self) -> String {
        if self.exists {
            format!("{} files, {} chunks", self.files_indexed, self.chunks)
        } else {
            "no index found".to_string()
        }
    }
}

/// Data passed forward from read_index — full metadata and statistics.
pub struct IndexData {
    /// Build metadata from parquet key-value metadata.
    pub metadata: BuildMetadata,
    /// Index statistics (file and chunk counts).
    pub stats: IndexStats,
}

/// Read index metadata and statistics from the `.mdvs/` directory.
///
/// Returns the step result and the full index data (for the command result).
/// "No index found" is a normal `Completed` output with `exists=false`.
pub fn run_read_index(path: &Path) -> (ProcessingStepResult<ReadIndexOutput>, Option<IndexData>) {
    let start = Instant::now();
    let backend = Backend::parquet(path);

    if !backend.exists() {
        let step = ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: ReadIndexOutput {
                exists: false,
                files_indexed: 0,
                chunks: 0,
            },
        };
        return (ProcessingStepResult::Completed(step), None);
    }

    let build_meta = match backend.read_metadata() {
        Ok(m) => m,
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            return (ProcessingStepResult::Failed(err), None);
        }
    };

    let idx_stats = match backend.stats() {
        Ok(s) => s,
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            return (ProcessingStepResult::Failed(err), None);
        }
    };

    match (build_meta, idx_stats) {
        (Some(metadata), Some(stats)) => {
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ReadIndexOutput {
                    exists: true,
                    files_indexed: stats.files_indexed,
                    chunks: stats.chunks,
                },
            };
            let data = IndexData { metadata, stats };
            (ProcessingStepResult::Completed(step), Some(data))
        }
        _ => {
            // Index directory exists but metadata/stats missing — treat as no index
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ReadIndexOutput {
                    exists: false,
                    files_indexed: 0,
                    chunks: 0,
                },
            };
            (ProcessingStepResult::Completed(step), None)
        }
    }
}
