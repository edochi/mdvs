//! Embed step of the build pipeline.
//!
//! Takes the [`FileToEmbed`](super::classify::FileToEmbed) set produced by
//! classification, chunks each file, extracts plain text, runs the embedder,
//! and produces [`ChunkRow`]s ready to write. Called from
//! [`super::build_core`].

use crate::discover::scan::ScannedFile;
use crate::index::chunk::{Chunks, extract_plain_text};
use crate::index::embed::Embedder;
use crate::index::storage::ChunkRow;
use crate::output::BuildFileDetail;
use anyhow::Context;

/// Data produced by the embed files step.
pub(super) struct EmbedFilesData {
    /// Chunk rows for newly embedded files.
    pub(super) chunk_rows: Vec<ChunkRow>,
    /// Per-file chunk counts (for verbose output).
    pub(super) details: Vec<BuildFileDetail>,
}

/// Chunk, extract plain text, embed, and produce chunk rows for a single file.
///
/// # Errors
///
/// Fails when a chunk index or line number does not fit the index's `i32`
/// columns.
pub(super) async fn embed_file(
    file_id: &str,
    file: &ScannedFile,
    max_chunk_size: usize,
    embedder: &Embedder,
) -> anyhow::Result<Vec<ChunkRow>> {
    let chunks = Chunks::new(&file.content, max_chunk_size);
    let plain_texts: Vec<String> = chunks
        .iter()
        .map(|c| extract_plain_text(&c.plain_text))
        .collect();
    let text_refs: Vec<&str> = plain_texts.iter().map(String::as_str).collect();
    let embeddings = if text_refs.is_empty() {
        vec![]
    } else {
        embedder.embed_batch(&text_refs).await
    };

    chunks
        .iter()
        .zip(embeddings)
        .zip(plain_texts)
        .map(|((chunk, embedding), chunk_text)| {
            let path = file.path.display();
            let start = chunk.start_line + file.body_line_offset;
            let end = chunk.end_line + file.body_line_offset;
            Ok(ChunkRow {
                chunk_id: uuid::Uuid::new_v4().to_string(),
                file_id: file_id.to_string(),
                chunk_index: i32::try_from(chunk.chunk_index).with_context(|| {
                    format!(
                        "{path}: chunk index {} exceeds the index limit of {}",
                        chunk.chunk_index,
                        i32::MAX
                    )
                })?,
                start_line: i32::try_from(start).with_context(|| {
                    format!(
                        "{path}: chunk start line {start} exceeds the index limit of {}",
                        i32::MAX
                    )
                })?,
                end_line: i32::try_from(end).with_context(|| {
                    format!(
                        "{path}: chunk end line {end} exceeds the index limit of {}",
                        i32::MAX
                    )
                })?,
                chunk_text,
                embedding,
            })
        })
        .collect()
}
