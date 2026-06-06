//! Read-side methods of [`LanceBackend`]: pulling rows + metadata back
//! out of an existing Lance index.
//!
//! Lives in its own file because `mdvs build` / `mdvs search` / `mdvs
//! info` share these readers (each opens the table, runs a projection,
//! and decodes batches into in-memory structs). The write path is in
//! [`super`] (`mod.rs`); the search path is in [`super::search`].

use super::{IndexStats, LanceBackend, i32_col, str_col};
use crate::index::storage::{
    BuildMetadata, COL_CHUNK_ID, COL_CHUNK_INDEX, COL_CHUNK_TEXT, COL_CONTENT_HASH, COL_EMBEDDING,
    COL_END_LINE, COL_FILE_ID, COL_FILEPATH, COL_START_LINE, ChunkRow, FileIndexEntry,
};
use arrow::array::{Array, FixedSizeListArray, Float32Array, RecordBatch};
use arrow::datatypes::DataType;
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use std::collections::HashSet;

impl LanceBackend {
    pub(super) async fn read_metadata(&self) -> anyhow::Result<Option<BuildMetadata>> {
        let Some(table) = self.open_table().await? else {
            return Ok(None);
        };
        let schema = table.schema().await?;
        Ok(BuildMetadata::from_hash_map(schema.metadata()))
    }

    pub(super) async fn read_file_index(&self) -> anyhow::Result<Vec<FileIndexEntry>> {
        let Some(table) = self.open_table().await? else {
            return Ok(vec![]);
        };
        let batches: Vec<RecordBatch> = table
            .query()
            .select(Select::columns(&[
                COL_FILE_ID,
                COL_FILEPATH,
                COL_CONTENT_HASH,
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        // Rows are per-chunk; dedupe to one entry per file_id (first wins).
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        for batch in &batches {
            let file_ids = str_col(batch, COL_FILE_ID)?;
            let filenames = str_col(batch, COL_FILEPATH)?;
            let hashes = str_col(batch, COL_CONTENT_HASH)?;
            for i in 0..batch.num_rows() {
                let file_id = file_ids.value(i).to_string();
                if seen.insert(file_id.clone()) {
                    entries.push(FileIndexEntry {
                        file_id,
                        filename: filenames.value(i).to_string(),
                        content_hash: hashes.value(i).to_string(),
                    });
                }
            }
        }
        Ok(entries)
    }

    pub(super) async fn read_chunk_rows(&self) -> anyhow::Result<Vec<ChunkRow>> {
        let Some(table) = self.open_table().await? else {
            return Ok(vec![]);
        };
        let batches: Vec<RecordBatch> = table
            .query()
            .select(Select::columns(&[
                COL_CHUNK_ID,
                COL_FILE_ID,
                COL_CHUNK_INDEX,
                COL_START_LINE,
                COL_END_LINE,
                COL_CHUNK_TEXT,
                COL_EMBEDDING,
            ]))
            .execute()
            .await?
            .try_collect()
            .await?;

        let mut rows = Vec::new();
        for batch in &batches {
            let chunk_ids = str_col(batch, COL_CHUNK_ID)?;
            let file_ids = str_col(batch, COL_FILE_ID)?;
            let chunk_indices = i32_col(batch, COL_CHUNK_INDEX)?;
            let start_lines = i32_col(batch, COL_START_LINE)?;
            let end_lines = i32_col(batch, COL_END_LINE)?;
            let chunk_texts = str_col(batch, COL_CHUNK_TEXT)?;
            let embeddings = batch
                .column_by_name(COL_EMBEDDING)
                .ok_or_else(|| anyhow::anyhow!("missing embedding column"))?
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .ok_or_else(|| anyhow::anyhow!("expected FixedSizeListArray for embedding"))?;
            for i in 0..batch.num_rows() {
                let emb = embeddings.value(i);
                let floats = emb
                    .as_any()
                    .downcast_ref::<Float32Array>()
                    .ok_or_else(|| anyhow::anyhow!("expected Float32Array in embedding"))?;
                let embedding: Vec<f32> = (0..floats.len()).map(|j| floats.value(j)).collect();
                rows.push(ChunkRow {
                    chunk_id: chunk_ids.value(i).to_string(),
                    file_id: file_ids.value(i).to_string(),
                    chunk_index: chunk_indices.value(i),
                    start_line: start_lines.value(i),
                    end_line: end_lines.value(i),
                    chunk_text: chunk_texts.value(i).to_string(),
                    embedding,
                });
            }
        }
        Ok(rows)
    }

    pub(super) async fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
        let Some(table) = self.open_table().await? else {
            return Ok(None);
        };
        let schema = table.schema().await?;
        if let Ok(field) = schema.field_with_name(COL_EMBEDDING)
            && let DataType::FixedSizeList(_, dim) = field.data_type()
        {
            return Ok(Some(*dim));
        }
        Ok(None)
    }

    pub(super) async fn stats(&self) -> anyhow::Result<Option<IndexStats>> {
        let Some(table) = self.open_table().await? else {
            return Ok(None);
        };
        let chunks = table.count_rows(None).await?;
        let files_indexed = self.read_file_index().await?.len();
        Ok(Some(IndexStats {
            files_indexed,
            chunks,
        }))
    }
}
