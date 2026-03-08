//! Embed step — embeds queries and files using the loaded model.

use serde::Serialize;
use std::time::Instant;

use crate::discover::scan::ScannedFile;
use crate::index::chunk::{extract_plain_text, Chunks};
use crate::index::embed::Embedder;
use crate::index::storage::ChunkRow;
use crate::output::format_file_count;
use crate::pipeline::classify::FileToEmbed;
use crate::pipeline::write_index::BuildFileDetail;
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

/// Output record for the embed files step.
#[derive(Debug, Serialize)]
pub struct EmbedFilesOutput {
    /// Number of files that were embedded.
    pub files_embedded: usize,
    /// Total number of chunks produced.
    pub chunks_produced: usize,
}

impl StepOutput for EmbedFilesOutput {
    fn format_line(&self) -> String {
        format!(
            "Embedded {} ({} chunks)",
            format_file_count(self.files_embedded),
            self.chunks_produced
        )
    }
}

/// Data produced by the embed files step.
pub(crate) struct EmbedFilesData {
    /// Chunk rows for newly embedded files.
    pub chunk_rows: Vec<ChunkRow>,
    /// Per-file chunk counts (for verbose output).
    pub details: Vec<BuildFileDetail>,
}

/// Embed a batch of files: chunk, extract plain text, embed, produce rows.
///
/// Returns the step result and the embed data for the write_index step.
pub(crate) async fn run_embed_files(
    files: &[FileToEmbed<'_>],
    embedder: &Embedder,
    max_chunk_size: usize,
) -> (
    ProcessingStepResult<EmbedFilesOutput>,
    Option<EmbedFilesData>,
) {
    let start = Instant::now();
    let mut chunk_rows = Vec::new();
    let mut details = Vec::new();

    for fte in files {
        let crs = embed_file(&fte.file_id, fte.scanned, max_chunk_size, embedder).await;
        details.push(BuildFileDetail {
            filename: fte.scanned.path.display().to_string(),
            chunks: crs.len(),
        });
        chunk_rows.extend(crs);
    }

    let step = ProcessingStep {
        elapsed_ms: start.elapsed().as_millis() as u64,
        output: EmbedFilesOutput {
            files_embedded: files.len(),
            chunks_produced: chunk_rows.len(),
        },
    };
    let data = EmbedFilesData {
        chunk_rows,
        details,
    };
    (ProcessingStepResult::Completed(step), Some(data))
}

/// Chunk, extract plain text, embed, and produce chunk rows for a single file.
async fn embed_file(
    file_id: &str,
    file: &ScannedFile,
    max_chunk_size: usize,
    embedder: &Embedder,
) -> Vec<ChunkRow> {
    let chunks = Chunks::new(&file.content, max_chunk_size);
    let plain_texts: Vec<String> = chunks
        .iter()
        .map(|c| extract_plain_text(&c.plain_text))
        .collect();
    let text_refs: Vec<&str> = plain_texts.iter().map(|s| s.as_str()).collect();
    let embeddings = if text_refs.is_empty() {
        vec![]
    } else {
        embedder.embed_batch(&text_refs).await
    };

    chunks
        .iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| ChunkRow {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            chunk_index: chunk.chunk_index as i32,
            start_line: chunk.start_line as i32,
            end_line: chunk.end_line as i32,
            embedding,
        })
        .collect()
}
