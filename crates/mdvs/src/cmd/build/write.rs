//! Index write step — the three-way decision (skip / full overwrite /
//! incremental delete+append) used by [`super::build_core`].
//!
//! [`WritePlan::decide`] picks the path; [`write_index_step`] carries it out.
//! The backend-side `write_index` / `write_index_incremental`
//! implementations live in [`crate::index::backend`].

use std::time::Instant;

use anyhow::Context;

use super::classify::ClassifyData;
use crate::{
    discover::{field_type::FieldType, scan::ScannedFiles},
    index::{
        backend::Backend,
        storage::{BuildMetadata, ChunkRow, FileRow, content_hash},
    },
    outcome::{Outcome, WriteIndexOutcome},
    step::{ErrorKind, StepEntry, elapsed_ms},
};

/// How the index write persists this build.
pub(super) enum WritePlan<'a> {
    /// Nothing to persist: not a full rebuild, no file removed, and no new
    /// chunk embedded.
    Skip,
    /// Recreate the table from scratch with every file row and chunk.
    Overwrite {
        file_rows: &'a [FileRow],
        chunk_rows: &'a [ChunkRow],
    },
    /// Delete the rows of `file_ids_to_clear`, append the new chunks (each
    /// joined with its file row from `file_rows`), replace the build
    /// metadata, and optimize the indexes. File rows without a new chunk
    /// are not written.
    Incremental {
        file_ids_to_clear: Vec<String>,
        file_rows: &'a [FileRow],
        new_chunk_rows: &'a [ChunkRow],
    },
}

impl<'a> WritePlan<'a> {
    /// Choose the write path for a classified build.
    ///
    /// A full rebuild always overwrites; it retains no chunks, so
    /// `new_chunk_rows` is the whole chunk table. Otherwise the write is
    /// skipped when no file was removed and no new chunk was embedded.
    ///
    /// The skip rule looks at `new_chunk_rows` (not at the files needing
    /// embedding) because empty-body files like Hugo `_index.md` are always
    /// classified as needing embedding — they have zero rows in the
    /// one-row-per-chunk index, so classify can't see them as unchanged —
    /// but they produce zero new chunks and the write would be a no-op.
    pub(super) fn decide(
        classify_data: &ClassifyData<'_>,
        file_rows: &'a [FileRow],
        new_chunk_rows: &'a [ChunkRow],
    ) -> Self {
        if classify_data.full_rebuild {
            return Self::Overwrite {
                file_rows,
                chunk_rows: new_chunk_rows,
            };
        }
        if classify_data.removed_count == 0 && new_chunk_rows.is_empty() {
            return Self::Skip;
        }
        // New and changed files plus removed files: their rows are deleted,
        // then the newly embedded chunks are appended.
        let file_ids_to_clear = classify_data
            .needs_embedding
            .iter()
            .map(|fte| fte.file_id.clone())
            .chain(classify_data.removed_file_ids.iter().cloned())
            .collect();
        Self::Incremental {
            file_ids_to_clear,
            file_rows,
            new_chunk_rows,
        }
    }
}

/// One file row per scanned file, under the `file_id` classification chose.
///
/// # Errors
///
/// Fails when a scanned file has no `file_id` in the classification.
pub(super) fn file_rows(
    scanned: &ScannedFiles,
    classify_data: &ClassifyData<'_>,
    built_at: i64,
) -> anyhow::Result<Vec<FileRow>> {
    scanned
        .files
        .iter()
        .map(|f| {
            let filename = f.path.display().to_string();
            let file_id = classify_data
                .file_id_map
                .get(&filename)
                .with_context(|| format!("no file id was assigned to '{filename}'"))?
                .clone();
            Ok(FileRow {
                file_id,
                filename,
                frontmatter: f.data.clone(),
                content_hash: content_hash(&f.content),
                built_at,
            })
        })
        .collect()
}

/// Carry out `plan`, timed as one step.
///
/// A skipped plan pushes a skipped step; a failed write pushes an
/// application error.
pub(super) async fn write_index_step(
    backend: &Backend,
    schema_fields: &[(String, FieldType)],
    plan: WritePlan<'_>,
    metadata: BuildMetadata,
    steps: &mut Vec<StepEntry>,
) -> Result<(), ()> {
    let write_start = Instant::now();
    let written = match plan {
        WritePlan::Skip => {
            steps.push(StepEntry::skipped());
            return Ok(());
        }
        WritePlan::Overwrite {
            file_rows,
            chunk_rows,
        } => backend
            .write_index(schema_fields, file_rows, chunk_rows, metadata)
            .await
            .map(|()| (file_rows.len(), chunk_rows.len())),
        WritePlan::Incremental {
            file_ids_to_clear,
            file_rows,
            new_chunk_rows,
        } => backend
            .write_index_incremental(
                schema_fields,
                &file_ids_to_clear,
                file_rows,
                new_chunk_rows,
                metadata,
            )
            .await
            .map(|()| (file_rows.len(), new_chunk_rows.len())),
    };
    match written {
        Ok((files_written, chunks_written)) => {
            steps.push(StepEntry::ok(
                Outcome::WriteIndex(WriteIndexOutcome {
                    files_written,
                    chunks_written,
                }),
                elapsed_ms(write_start),
            ));
            Ok(())
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(write_start),
            ));
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, path::PathBuf};

    use super::*;
    use crate::{cmd::build::classify::FileToEmbed, discover::scan::ScannedFile};

    fn scanned_file(path: &str, body: &str) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(path),
            data: None,
            content: body.to_string(),
            body_line_offset: 0,
            frontmatter_error: None,
        }
    }

    fn file_row(file_id: &str) -> FileRow {
        FileRow {
            file_id: file_id.into(),
            filename: "a.md".into(),
            frontmatter: None,
            content_hash: content_hash("body"),
            built_at: 0,
        }
    }

    fn chunk(file_id: &str) -> ChunkRow {
        ChunkRow {
            chunk_id: "c1".into(),
            file_id: file_id.into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            chunk_text: "text".into(),
            embedding: vec![],
        }
    }

    /// A classification with `files` needing embedding and `removed` file
    /// ids removed since the previous build.
    fn classified<'a>(
        full_rebuild: bool,
        files: &'a [ScannedFile],
        removed: &[&str],
    ) -> ClassifyData<'a> {
        ClassifyData {
            full_rebuild,
            needs_embedding: files
                .iter()
                .enumerate()
                .map(|(i, f)| FileToEmbed {
                    file_id: format!("new-{i}"),
                    scanned: f,
                })
                .collect(),
            file_id_map: HashMap::new(),
            retained_chunks: vec![],
            unchanged_count: 0,
            removed_count: removed.len(),
            chunks_removed: 0,
            removed_details: vec![],
            removed_file_ids: removed.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn decide_overwrites_full_rebuild_even_without_chunks() {
        let data = classified(true, &[], &[]);
        let plan = WritePlan::decide(&data, &[], &[]);
        assert!(matches!(plan, WritePlan::Overwrite { .. }));
    }

    #[test]
    fn decide_overwrite_carries_the_given_rows() {
        let data = classified(true, &[], &[]);
        let rows = [file_row("new-0")];
        let chunks = [chunk("new-0"), chunk("new-0")];
        let WritePlan::Overwrite {
            file_rows,
            chunk_rows,
        } = WritePlan::decide(&data, &rows, &chunks)
        else {
            panic!("expected an overwrite");
        };
        assert_eq!(
            file_rows.iter().map(|r| &r.file_id).collect::<Vec<_>>(),
            vec!["new-0"]
        );
        assert_eq!(chunk_rows.len(), chunks.len());
        assert!(std::ptr::eq(chunk_rows, chunks.as_slice()));
        assert!(std::ptr::eq(file_rows, rows.as_slice()));
    }

    #[test]
    fn decide_clears_embedded_and_removed_files_together() {
        let files = [scanned_file("a.md", "body")];
        let data = classified(false, &files, &["gone"]);
        let new_chunks = [chunk("new-0")];
        let WritePlan::Incremental {
            file_ids_to_clear, ..
        } = WritePlan::decide(&data, &[], &new_chunks)
        else {
            panic!("expected an incremental write");
        };
        assert_eq!(
            file_ids_to_clear,
            vec!["new-0".to_string(), "gone".to_string()]
        );
    }

    #[test]
    fn decide_skips_when_nothing_embedded_and_nothing_removed() {
        let data = classified(false, &[], &[]);
        assert!(matches!(
            WritePlan::decide(&data, &[], &[]),
            WritePlan::Skip
        ));
    }

    #[test]
    fn decide_skips_empty_body_file_that_produced_no_chunks() {
        // An empty-body file is always classified as needing embedding but
        // yields no chunks, so there is nothing to write.
        let files = [scanned_file("_index.md", "")];
        let data = classified(false, &files, &[]);
        assert!(matches!(
            WritePlan::decide(&data, &[], &[]),
            WritePlan::Skip
        ));
    }

    #[test]
    fn decide_writes_incrementally_when_chunks_were_embedded() {
        let files = [scanned_file("a.md", "body")];
        let data = classified(false, &files, &[]);
        let new_chunks = [chunk("new-0")];
        let plan = WritePlan::decide(&data, &[], &new_chunks);
        let WritePlan::Incremental {
            file_ids_to_clear,
            new_chunk_rows,
            ..
        } = plan
        else {
            panic!("expected an incremental write");
        };
        assert_eq!(file_ids_to_clear, vec!["new-0".to_string()]);
        assert_eq!(new_chunk_rows.len(), new_chunks.len());
    }

    #[test]
    fn decide_writes_incrementally_when_a_file_was_removed() {
        let data = classified(false, &[], &["gone"]);
        let plan = WritePlan::decide(&data, &[], &[]);
        let WritePlan::Incremental {
            file_ids_to_clear, ..
        } = plan
        else {
            panic!("expected an incremental write");
        };
        assert_eq!(file_ids_to_clear, vec!["gone".to_string()]);
    }
}
