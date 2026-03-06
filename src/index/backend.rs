use crate::discover::field_type::FieldType;
use crate::index::storage::{
    build_chunks_batch, build_files_batch, col, read_build_metadata, read_chunk_rows,
    read_file_index, read_parquet, write_parquet, write_parquet_with_metadata, BuildMetadata,
    ChunkRow, FileIndexEntry, FileRow,
};
use crate::search::SearchContext;
use datafusion::arrow::array::{Array, Float64Array, Int32Array, StringViewArray};
use datafusion::arrow::datatypes::DataType;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// A single search result with its relevance score.
#[derive(Debug, Serialize)]
pub struct SearchHit {
    /// Path of the matching file relative to the project root.
    pub filename: String,
    /// Cosine similarity score (higher is more relevant).
    pub score: f64,
    /// Start line of the best matching chunk (1-indexed).
    pub start_line: Option<i32>,
    /// End line of the best matching chunk (1-indexed).
    pub end_line: Option<i32>,
    /// Text of the best matching chunk (populated in verbose mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk_text: Option<String>,
}

/// Summary statistics for a built search index.
#[derive(Debug, Serialize)]
pub struct IndexStats {
    /// Number of files in the index.
    pub files_indexed: usize,
    /// Total number of chunks across all files.
    pub chunks: usize,
}

pub(crate) enum Backend {
    Parquet(ParquetBackend),
}

impl Backend {
    pub(crate) fn parquet(root: &Path, prefix: &str) -> Self {
        Backend::Parquet(ParquetBackend {
            root: root.to_path_buf(),
            prefix: prefix.to_string(),
        })
    }

    #[instrument(name = "write_index", skip_all)]
    pub fn write_index(
        &self,
        schema_fields: &[(String, FieldType)],
        files: &[FileRow],
        chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        match self {
            Backend::Parquet(b) => b.write_index(schema_fields, files, chunks, metadata),
        }
    }

    #[instrument(name = "read_metadata", skip_all, level = "debug")]
    pub fn read_metadata(&self) -> anyhow::Result<Option<BuildMetadata>> {
        match self {
            Backend::Parquet(b) => b.read_metadata(),
        }
    }

    #[instrument(name = "read_file_index", skip_all, level = "debug")]
    pub fn read_file_index(&self) -> anyhow::Result<Vec<FileIndexEntry>> {
        match self {
            Backend::Parquet(b) => b.read_file_index(),
        }
    }

    #[instrument(name = "read_chunk_rows", skip_all, level = "debug")]
    pub fn read_chunk_rows(&self) -> anyhow::Result<Vec<ChunkRow>> {
        match self {
            Backend::Parquet(b) => b.read_chunk_rows(),
        }
    }

    #[instrument(name = "embedding_dimension", skip_all, level = "debug")]
    pub fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
        match self {
            Backend::Parquet(b) => b.embedding_dimension(),
        }
    }

    #[instrument(name = "search_index", skip_all)]
    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
        where_clause: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        match self {
            Backend::Parquet(b) => b.search(query_embedding, where_clause, limit).await,
        }
    }

    #[instrument(name = "stats", skip_all, level = "debug")]
    pub fn stats(&self) -> anyhow::Result<Option<IndexStats>> {
        match self {
            Backend::Parquet(b) => b.stats(),
        }
    }

    pub fn exists(&self) -> bool {
        match self {
            Backend::Parquet(b) => b.exists(),
        }
    }

    #[instrument(name = "clean_index", skip_all)]
    pub fn clean(&self) -> anyhow::Result<()> {
        match self {
            Backend::Parquet(b) => b.clean(),
        }
    }
}

pub(crate) struct ParquetBackend {
    root: PathBuf,
    prefix: String,
}

impl ParquetBackend {
    fn mdvs_dir(&self) -> PathBuf {
        self.root.join(".mdvs")
    }

    fn files_parquet(&self) -> PathBuf {
        self.mdvs_dir().join("files.parquet")
    }

    fn chunks_parquet(&self) -> PathBuf {
        self.mdvs_dir().join("chunks.parquet")
    }

    fn write_index(
        &self,
        schema_fields: &[(String, FieldType)],
        files: &[FileRow],
        chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.mdvs_dir())?;

        let files_batch = build_files_batch(schema_fields, files, &self.prefix);
        write_parquet_with_metadata(
            &self.files_parquet(),
            &files_batch,
            metadata.to_hash_map(),
        )?;

        let dimension = chunks.first().map(|c| c.embedding.len() as i32).unwrap_or(0);
        let chunks_batch = build_chunks_batch(chunks, dimension, &self.prefix);
        write_parquet(&self.chunks_parquet(), &chunks_batch)?;

        Ok(())
    }

    fn read_metadata(&self) -> anyhow::Result<Option<BuildMetadata>> {
        if !self.files_parquet().exists() {
            return Ok(None);
        }
        read_build_metadata(&self.files_parquet())
    }

    fn read_file_index(&self) -> anyhow::Result<Vec<FileIndexEntry>> {
        if !self.files_parquet().exists() {
            return Ok(vec![]);
        }
        read_file_index(&self.files_parquet())
    }

    fn read_chunk_rows(&self) -> anyhow::Result<Vec<ChunkRow>> {
        if !self.chunks_parquet().exists() {
            return Ok(vec![]);
        }
        read_chunk_rows(&self.chunks_parquet())
    }

    fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
        if !self.chunks_parquet().exists() {
            return Ok(None);
        }
        let batches = read_parquet(&self.chunks_parquet())?;
        if let Some(batch) = batches.first() {
            if let Ok(field) = batch.schema().field_with_name(&col(&self.prefix, "embedding")) {
                if let DataType::FixedSizeList(_, dim) = field.data_type() {
                    return Ok(Some(*dim));
                }
            }
        }
        Ok(None)
    }

    async fn search(
        &self,
        query_embedding: Vec<f32>,
        where_clause: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        let sc = SearchContext::new(
            &self.files_parquet(),
            &self.chunks_parquet(),
            query_embedding,
            &self.prefix,
        )
        .await?;

        let where_part = match where_clause {
            Some(w) => format!("AND {w}"),
            None => String::new(),
        };
        let p = &self.prefix;
        let sql = format!(
            "SELECT f.{fn_col}, sub.score, sub.{sl_col}, sub.{el_col}
             FROM (
                 SELECT c.{c_fid},
                        cosine_similarity(c.{emb_col}) AS score,
                        c.{sl_col},
                        c.{el_col},
                        ROW_NUMBER() OVER (
                            PARTITION BY c.{c_fid}
                            ORDER BY cosine_similarity(c.{emb_col}) DESC
                        ) AS rn
                 FROM chunks c
             ) sub
             JOIN files_v f ON sub.{c_fid} = f.{f_fid}
             WHERE sub.rn = 1
             {where_part}
             ORDER BY sub.score DESC
             LIMIT {limit}",
            fn_col = col(p, "filename"),
            emb_col = col(p, "embedding"),
            c_fid = col(p, "file_id"),
            f_fid = col(p, "file_id"),
            sl_col = col(p, "start_line"),
            el_col = col(p, "end_line"),
        );

        let batches = sc.query(&sql).await?;

        let mut hits = Vec::new();
        for batch in &batches {
            let filenames = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringViewArray>()
                .ok_or_else(|| anyhow::anyhow!("unexpected type for filename column"))?;
            let scores = batch
                .column(1)
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(|| anyhow::anyhow!("unexpected type for score column"))?;
            let start_lines = batch
                .column(2)
                .as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(|| anyhow::anyhow!("unexpected type for start_line column"))?;
            let end_lines = batch
                .column(3)
                .as_any()
                .downcast_ref::<Int32Array>()
                .ok_or_else(|| anyhow::anyhow!("unexpected type for end_line column"))?;

            for i in 0..batch.num_rows() {
                hits.push(SearchHit {
                    filename: filenames.value(i).to_string(),
                    score: scores.value(i),
                    start_line: if start_lines.is_null(i) { None } else { Some(start_lines.value(i)) },
                    end_line: if end_lines.is_null(i) { None } else { Some(end_lines.value(i)) },
                    chunk_text: None,
                });
            }
        }

        Ok(hits)
    }

    fn stats(&self) -> anyhow::Result<Option<IndexStats>> {
        if !self.exists() {
            return Ok(None);
        }
        let file_batches = read_parquet(&self.files_parquet())?;
        let chunk_batches = read_parquet(&self.chunks_parquet())?;
        let files_indexed: usize = file_batches.iter().map(|b| b.num_rows()).sum();
        let chunks: usize = chunk_batches.iter().map(|b| b.num_rows()).sum();
        Ok(Some(IndexStats {
            files_indexed,
            chunks,
        }))
    }

    fn exists(&self) -> bool {
        self.files_parquet().exists() && self.chunks_parquet().exists()
    }

    fn clean(&self) -> anyhow::Result<()> {
        let dir = self.mdvs_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};

    fn test_schema_fields() -> Vec<(String, FieldType)> {
        vec![
            ("title".into(), FieldType::String),
            ("draft".into(), FieldType::Boolean),
        ]
    }

    fn test_files() -> Vec<FileRow> {
        vec![
            FileRow {
                file_id: "f1".into(),
                filename: "blog/rust.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Rust Guide", "draft": false})),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "f2".into(),
                filename: "blog/python.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Python Intro", "draft": false})),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            },
        ]
    }

    fn test_chunks() -> Vec<ChunkRow> {
        vec![
            ChunkRow {
                chunk_id: "c1".into(),
                file_id: "f1".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 4,
                embedding: vec![0.9, 0.1, 0.0, 0.0],
            },
            ChunkRow {
                chunk_id: "c2".into(),
                file_id: "f2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.1, 0.9, 0.0, 0.0],
            },
        ]
    }

    fn test_metadata() -> BuildMetadata {
        BuildMetadata {
            embedding_model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "test-model".into(),
                revision: Some("abc123".into()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            glob: "**".into(),
            built_at: "2026-03-02T12:00:00+00:00".into(),
            internal_prefix: "_".into(),
        }
    }

    #[test]
    fn write_and_read_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();

        let meta = backend.read_metadata().unwrap();
        assert_eq!(meta, Some(test_metadata()));
    }

    #[test]
    fn write_and_stats() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();

        let stats = backend.stats().unwrap().unwrap();
        assert_eq!(stats.files_indexed, 2);
        assert_eq!(stats.chunks, 2);
    }

    #[test]
    fn exists_false_then_true() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        assert!(!backend.exists());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();

        assert!(backend.exists());
    }

    #[test]
    fn clean_removes_index() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();
        assert!(backend.exists());

        backend.clean().unwrap();
        assert!(!backend.exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn embedding_dimension_correct() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        assert_eq!(backend.embedding_dimension().unwrap(), None);

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();

        assert_eq!(backend.embedding_dimension().unwrap(), Some(4));
    }

    #[tokio::test]
    async fn search_returns_hits() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path(), "_");

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .unwrap();

        // Query vector close to rust.md's embedding
        let hits = backend
            .search(vec![1.0, 0.0, 0.0, 0.0], None, 10)
            .await
            .unwrap();

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].filename, "blog/rust.md");
        assert!(hits[0].score > hits[1].score);
        assert_eq!(hits[0].start_line, Some(1));
        assert_eq!(hits[0].end_line, Some(4));
    }
}
