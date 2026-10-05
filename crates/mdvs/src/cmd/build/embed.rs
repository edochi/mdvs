//! Embed step of the build pipeline.
//!
//! Takes the [`FileToEmbed`](super::classify::FileToEmbed) set produced by
//! classification, chunks each file, extracts plain text, runs the embedder,
//! and produces [`ChunkRow`]s ready to write. Called from
//! [`super::build_core`].

use std::time::Instant;

use anyhow::Context;

use super::classify::{ClassifyData, FileToEmbed};
use crate::{
    cmd::steps::load_model_step,
    discover::scan::ScannedFile,
    index::{
        backend::Backend,
        chunk::{Chunks, extract_plain_text},
        embed::Embedder,
        storage::ChunkRow,
    },
    outcome::{EmbedFilesOutcome, Outcome},
    output::BuildFileDetail,
    schema::shared::EmbeddingModelConfig,
    step::{ErrorKind, StepEntry, elapsed_ms},
};

/// Data produced by the embed files step.
pub(super) struct EmbedFilesData {
    /// Chunk rows for newly embedded files.
    pub(super) chunk_rows: Vec<ChunkRow>,
    /// Per-file chunk counts (for verbose output).
    pub(super) details: Vec<BuildFileDetail>,
}

/// Load the embedding model when any file needs embedding, and check its
/// dimension against the existing index on an incremental build.
///
/// Pushes a skipped step and returns `None` when nothing needs embedding.
/// A failed load is followed by a "model loading failed" step; a dimension
/// mismatch is reported as a user error.
pub(super) async fn load_embedder_step(
    embedding: &EmbeddingModelConfig,
    backend: &Backend,
    classify_data: &ClassifyData<'_>,
    steps: &mut Vec<StepEntry>,
) -> Result<Option<Embedder>, ()> {
    if classify_data.needs_embedding.is_empty() {
        steps.push(StepEntry::skipped());
        return Ok(None);
    }
    let Ok(embedder) = load_model_step(embedding, steps) else {
        steps.push(StepEntry::err(
            ErrorKind::Application,
            "model loading failed".into(),
            0,
        ));
        return Err(());
    };
    if !classify_data.full_rebuild
        && let Some(msg) = check_dimension(backend, &embedder).await
    {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return Err(());
    }
    Ok(Some(embedder))
}

/// Compare the model's embedding dimension with the one stored in the index.
///
/// Returns the error message on a mismatch or a failed read, and `None` when
/// they match or the index records no dimension.
async fn check_dimension(backend: &Backend, embedder: &Embedder) -> Option<String> {
    match backend.embedding_dimension().await {
        Ok(Some(existing_dim)) => {
            let model_dim = embedder.dimension();
            // A stored dimension that is not a valid usize can never match
            // the model, so it counts as a mismatch.
            if usize::try_from(existing_dim).ok() == Some(model_dim) {
                None
            } else {
                Some(format!(
                    "dimension mismatch: model produces {model_dim}-dim embeddings but existing index has {existing_dim}-dim"
                ))
            }
        }
        Ok(None) => None,
        Err(e) => Some(e.to_string()),
    }
}

/// Chunk and embed every file in `files`, timed as one step.
///
/// The first file that fails stops the step with an untimed application
/// error.
pub(super) async fn embed_step(
    files: &[FileToEmbed<'_>],
    max_chunk_size: usize,
    embedder: &Embedder,
    steps: &mut Vec<StepEntry>,
) -> Result<EmbedFilesData, ()> {
    let embed_start = Instant::now();
    let mut chunk_rows = Vec::new();
    let mut details = Vec::new();
    for fte in files {
        let crs = embed_file(&fte.file_id, fte.scanned, max_chunk_size, embedder)
            .await
            .map_err(|e| {
                steps.push(StepEntry::err(ErrorKind::Application, format!("{e:#}"), 0));
            })?;
        details.push(BuildFileDetail {
            filename: fte.scanned.path.display().to_string(),
            chunks: crs.len(),
        });
        chunk_rows.extend(crs);
    }
    steps.push(StepEntry::ok(
        Outcome::EmbedFiles(EmbedFilesOutcome {
            files_embedded: files.len(),
            chunks_produced: chunk_rows.len(),
        }),
        elapsed_ms(embed_start),
    ));
    Ok(EmbedFilesData {
        chunk_rows,
        details,
    })
}

/// Chunk, extract plain text, embed, and produce chunk rows for a single file.
///
/// # Errors
///
/// Fails when a chunk index or line number does not fit the index's `i32`
/// columns.
async fn embed_file(
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
