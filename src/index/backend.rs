use crate::discover::field_type::FieldType;
use crate::index::storage::{
    build_chunks_batch, build_files_batch, read_build_metadata, read_chunk_rows, read_file_index,
    read_parquet, write_parquet, write_parquet_with_metadata, BuildMetadata, ChunkRow,
    FileIndexEntry, FileRow,
};
use crate::search::SearchContext;
use datafusion::arrow::array::{Array, Float64Array, StringViewArray};
use datafusion::arrow::datatypes::DataType;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::instrument;

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub filename: String,
    pub score: f64,
}

#[derive(Debug, Serialize)]
pub struct IndexStats {
    pub files_indexed: usize,
    pub chunks: usize,
}

pub(crate) enum Backend {
    Parquet(ParquetBackend),
}

impl Backend {
    pub(crate) fn parquet(root: &Path) -> Self {
        Backend::Parquet(ParquetBackend {
            root: root.to_path_buf(),
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

    pub fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
        match self {
            Backend::Parquet(b) => b.embedding_dimension(),
        }
    }

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

    pub fn clean(&self) -> anyhow::Result<()> {
        match self {
            Backend::Parquet(b) => b.clean(),
        }
    }
}

pub(crate) struct ParquetBackend {
    root: PathBuf,
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

        let files_batch = build_files_batch(schema_fields, files);
        write_parquet_with_metadata(
            &self.files_parquet(),
            &files_batch,
            metadata.to_hash_map(),
        )?;

        let dimension = chunks.first().map(|c| c.embedding.len() as i32).unwrap_or(0);
        let chunks_batch = build_chunks_batch(chunks, dimension);
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
        if let Some(batch) = batches.first()
            && let Ok(field) = batch.schema().field_with_name("embedding")
            && let DataType::FixedSizeList(_, dim) = field.data_type()
        {
            Ok(Some(*dim))
        } else {
            Ok(None)
        }
    }

    async fn search(
        &self,
        query_embedding: Vec<f32>,
        where_clause: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SearchHit>> {
        let sc =
            SearchContext::new(&self.files_parquet(), &self.chunks_parquet(), query_embedding)
                .await?;

        let where_part = match where_clause {
            Some(w) => format!("WHERE {w}"),
            None => String::new(),
        };
        let sql = format!(
            "SELECT f.filename,
                    MAX(cosine_similarity(c.embedding)) AS score
             FROM chunks c JOIN files f ON c.file_id = f.file_id
             {where_part}
             GROUP BY f.file_id, f.filename
             ORDER BY score DESC
             LIMIT {limit}"
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

            for i in 0..batch.num_rows() {
                hits.push(SearchHit {
                    filename: filenames.value(i).to_string(),
                    score: scores.value(i),
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
        }
    }

    #[test]
    fn write_and_read_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::parquet(tmp.path());

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
        let backend = Backend::parquet(tmp.path());

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
        let backend = Backend::parquet(tmp.path());

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
        let backend = Backend::parquet(tmp.path());

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
        let backend = Backend::parquet(tmp.path());

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
        let backend = Backend::parquet(tmp.path());

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
    }
}
