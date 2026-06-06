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

/// Data produced by the embed files step.
pub(super) struct EmbedFilesData {
    /// Chunk rows for newly embedded files.
    pub(super) chunk_rows: Vec<ChunkRow>,
    /// Per-file chunk counts (for verbose output).
    pub(super) details: Vec<BuildFileDetail>,
}

/// Chunk, extract plain text, embed, and produce chunk rows for a single file.
pub(super) async fn embed_file(
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
        .zip(plain_texts)
        .map(|((chunk, embedding), chunk_text)| ChunkRow {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            chunk_index: chunk.chunk_index as i32,
            start_line: (chunk.start_line + file.body_line_offset) as i32,
            end_line: (chunk.end_line + file.body_line_offset) as i32,
            chunk_text,
            embedding,
        })
        .collect()
}
