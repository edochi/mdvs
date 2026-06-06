//! Index write step — the three-way decision (skip / full overwrite /
//! incremental delete+append) used by [`super::build_core`].
//!
//! The dispatch lives here; the backend-side `write_index` /
//! `write_index_incremental` implementations live in
//! [`crate::index::backend`].

use crate::discover::field_type::FieldType;
use crate::index::backend::Backend;
use crate::index::storage::{BuildMetadata, ChunkRow, FileRow};

/// Result of [`write_index_step`]: either the write was skipped (because
/// nothing changed) or it ran to completion with the rendered counts.
pub(super) enum WriteOutcome {
    /// No write happened — nothing to persist and not a full rebuild.
    Skipped,
    /// Write succeeded, with the persisted row counts.
    Written {
        files_written: usize,
        chunks_written: usize,
    },
}

/// Dispatch the index write across three branches:
/// - **Skip** when not a full rebuild AND no files were removed AND no new
///   chunks were embedded (e.g. unchanged corpus, or only empty-body files
///   touched).
/// - **Full overwrite** when `full_rebuild` is true — the table is recreated
///   from scratch.
/// - **Incremental** otherwise — delete rows for `file_ids_to_clear`,
///   append the new chunks (the slice of `chunk_rows` past
///   `retained_chunks_count`), refresh metadata, optimize.
///
/// The skip check looks at `new_chunks_count` (not the size of the file
/// list) because empty-body files like Hugo `_index.md` are always marked
/// as needing embedding — they have zero rows in the one-row-per-chunk
/// index, so classify can't see them as unchanged — but they produce zero
/// new chunks at embed time and the write would be a no-op.
///
/// The caller pre-builds `file_ids_to_clear` from
/// `classify_data.needs_embedding` + `classify_data.removed_file_ids` so
/// this function doesn't need a `ClassifyData` borrow (which would conflict
/// with the caller having already moved `retained_chunks` out of it).
#[allow(clippy::too_many_arguments)]
pub(super) async fn write_index_step(
    backend: &Backend,
    schema_fields: &[(String, FieldType)],
    file_rows: &[FileRow],
    chunk_rows: &[ChunkRow],
    new_chunks_count: usize,
    full_rebuild: bool,
    removed_count: usize,
    file_ids_to_clear: &[String],
    retained_chunks_count: usize,
    metadata: BuildMetadata,
) -> anyhow::Result<WriteOutcome> {
    let nothing_to_persist = !full_rebuild && removed_count == 0 && new_chunks_count == 0;

    if nothing_to_persist {
        return Ok(WriteOutcome::Skipped);
    }

    if full_rebuild {
        backend
            .write_index(schema_fields, file_rows, chunk_rows, metadata)
            .await?;
        return Ok(WriteOutcome::Written {
            files_written: file_rows.len(),
            chunks_written: chunk_rows.len(),
        });
    }

    // Incremental path. `file_ids_to_clear` covers new+changed + removed
    // files; their rows are deleted, then the newly embedded chunks are
    // appended.
    let new_chunk_slice = &chunk_rows[retained_chunks_count..];
    backend
        .write_index_incremental(
            schema_fields,
            file_ids_to_clear,
            file_rows,
            new_chunk_slice,
            metadata,
        )
        .await?;
    Ok(WriteOutcome::Written {
        files_written: file_rows.len(),
        chunks_written: new_chunk_slice.len(),
    })
}
