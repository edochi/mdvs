//! Write index step — writes file and chunk rows to the Parquet index.

use serde::Serialize;
use std::time::Instant;

use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{BuildMetadata, ChunkRow, FileRow};
use crate::output::format_file_count;
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};

// BuildFileDetail moved to crate::output
pub use crate::output::BuildFileDetail;

/// Output record for the write index step.
#[derive(Debug, Serialize)]
pub struct WriteIndexOutput {
    /// Number of files written to the index.
    pub files_written: usize,
    /// Number of chunks written to the index.
    pub chunks_written: usize,
}

impl StepOutput for WriteIndexOutput {
    fn format_line(&self) -> String {
        format!(
            "{}, {} chunks",
            format_file_count(self.files_written),
            self.chunks_written
        )
    }
}

/// Write file and chunk rows to the Parquet index.
///
/// Returns the step result.
pub(crate) fn run_write_index(
    backend: &Backend,
    schema_fields: &[(String, FieldType)],
    file_rows: &[FileRow],
    chunk_rows: &[ChunkRow],
    metadata: BuildMetadata,
) -> ProcessingStepResult<WriteIndexOutput> {
    let start = Instant::now();
    match backend.write_index(schema_fields, file_rows, chunk_rows, metadata) {
        Ok(()) => ProcessingStepResult::Completed(ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: WriteIndexOutput {
                files_written: file_rows.len(),
                chunks_written: chunk_rows.len(),
            },
        }),
        Err(e) => ProcessingStepResult::Failed(ProcessingStepError {
            kind: ErrorKind::Application,
            message: e.to_string(),
        }),
    }
}
