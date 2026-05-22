use crate::discover::field_type::FieldType;
use crate::index::storage::{
    BuildMetadata, COL_CHUNK_ID, COL_CHUNK_INDEX, COL_CHUNK_TEXT, COL_CONTENT_HASH, COL_EMBEDDING,
    COL_END_LINE, COL_FILE_ID, COL_FILEPATH, COL_START_LINE, ChunkRow, FileIndexEntry, FileRow,
    build_index_batch,
};
use anyhow::Context;
use arrow::array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow::datatypes::DataType;
use futures::TryStreamExt;
use lancedb::database::CreateTableMode;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Name of the single denormalized Lance table.
const LANCE_TABLE: &str = "index";

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

/// Index backend. Single implementation (`LanceBackend`); kept as an enum
/// for the test seam and the project's enum-dispatch convention.
pub(crate) enum Backend {
    /// Lance + LanceDB backend (TODO-0016).
    Lance(LanceBackend),
}

impl Backend {
    pub(crate) fn lance(root: &Path) -> Self {
        Backend::Lance(LanceBackend {
            root: root.to_path_buf(),
        })
    }

    #[instrument(name = "write_index", skip_all)]
    pub async fn write_index(
        &self,
        schema_fields: &[(String, FieldType)],
        files: &[FileRow],
        chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        match self {
            Backend::Lance(b) => b.write_index(schema_fields, files, chunks, metadata).await,
        }
    }

    #[instrument(name = "read_metadata", skip_all, level = "debug")]
    pub async fn read_metadata(&self) -> anyhow::Result<Option<BuildMetadata>> {
        match self {
            Backend::Lance(b) => b.read_metadata().await,
        }
    }

    #[instrument(name = "read_file_index", skip_all, level = "debug")]
    pub async fn read_file_index(&self) -> anyhow::Result<Vec<FileIndexEntry>> {
        match self {
            Backend::Lance(b) => b.read_file_index().await,
        }
    }

    #[instrument(name = "read_chunk_rows", skip_all, level = "debug")]
    pub async fn read_chunk_rows(&self) -> anyhow::Result<Vec<ChunkRow>> {
        match self {
            Backend::Lance(b) => b.read_chunk_rows().await,
        }
    }

    #[instrument(name = "embedding_dimension", skip_all, level = "debug")]
    pub async fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
        match self {
            Backend::Lance(b) => b.embedding_dimension().await,
        }
    }

    #[instrument(name = "search_index", skip_all)]
    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
        where_clause: Option<&str>,
        limit: usize,
        internal_prefix: &str,
        aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        match self {
            Backend::Lance(b) => {
                b.search(
                    query_embedding,
                    where_clause,
                    limit,
                    internal_prefix,
                    aliases,
                )
                .await
            }
        }
    }

    #[instrument(name = "stats", skip_all, level = "debug")]
    pub async fn stats(&self) -> anyhow::Result<Option<IndexStats>> {
        match self {
            Backend::Lance(b) => b.stats().await,
        }
    }

    pub fn exists(&self) -> bool {
        match self {
            Backend::Lance(b) => b.exists(),
        }
    }

    #[instrument(name = "clean_index", skip_all)]
    pub async fn clean(&self) -> anyhow::Result<()> {
        match self {
            Backend::Lance(b) => b.clean(),
        }
    }
}

pub(crate) struct LanceBackend {
    root: PathBuf,
}

impl LanceBackend {
    fn db_dir(&self) -> PathBuf {
        self.root.join(".mdvs")
    }

    fn table_dir(&self) -> PathBuf {
        self.db_dir().join("index.lance")
    }

    async fn connect(&self) -> anyhow::Result<lancedb::Connection> {
        let uri = self
            .db_dir()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF8 path: {}", self.db_dir().display()))?
            .to_string();
        lancedb::connect(&uri)
            .execute()
            .await
            .context("connecting to Lance database")
    }

    /// Open the index table, or `None` if it doesn't exist yet.
    async fn open_table(&self) -> anyhow::Result<Option<lancedb::table::Table>> {
        if !self.table_dir().exists() {
            return Ok(None);
        }
        let conn = self.connect().await?;
        match conn.open_table(LANCE_TABLE).execute().await {
            Ok(t) => Ok(Some(t)),
            Err(lancedb::Error::TableNotFound { .. }) => Ok(None),
            Err(e) => Err(e).context("opening Lance index table"),
        }
    }

    async fn write_index(
        &self,
        schema_fields: &[(String, FieldType)],
        files: &[FileRow],
        chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        std::fs::create_dir_all(self.db_dir())?;

        let batch = build_index_batch(schema_fields, files, chunks)?;
        // Bake the seven mdvs.* build-metadata keys into the schema (spike #5).
        let schema_md = (*batch.schema())
            .clone()
            .with_metadata(metadata.to_hash_map());
        let schema_md = std::sync::Arc::new(schema_md);
        let batch = batch.with_schema(schema_md.clone())?;
        let reader: Box<dyn RecordBatchReader + Send> =
            Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema_md));

        let conn = self.connect().await?;
        conn.create_table(LANCE_TABLE, reader)
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await
            .context("creating Lance index table")?;
        Ok(())
    }

    async fn read_metadata(&self) -> anyhow::Result<Option<BuildMetadata>> {
        let Some(table) = self.open_table().await? else {
            return Ok(None);
        };
        let schema = table.schema().await?;
        Ok(BuildMetadata::from_hash_map(schema.metadata()))
    }

    async fn read_file_index(&self) -> anyhow::Result<Vec<FileIndexEntry>> {
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

    async fn read_chunk_rows(&self) -> anyhow::Result<Vec<ChunkRow>> {
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

    async fn embedding_dimension(&self) -> anyhow::Result<Option<i32>> {
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

    async fn stats(&self) -> anyhow::Result<Option<IndexStats>> {
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

    fn exists(&self) -> bool {
        self.table_dir().exists()
    }

    fn clean(&self) -> anyhow::Result<()> {
        let dir = self.db_dir();
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Throwaway brute-force cosine search (TODO-0016 wave 1 only; deleted in
    /// wave 2 in favour of native LanceDB ANN + hybrid). Reads candidate rows
    /// (optionally filtered via `--where`), scores cosine in Rust, keeps the
    /// best chunk per file, returns the top-K.
    async fn search(
        &self,
        query_embedding: Vec<f32>,
        where_clause: Option<&str>,
        limit: usize,
        _internal_prefix: &str,
        _aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        let Some(table) = self.open_table().await? else {
            return Ok(vec![]);
        };

        let mut query = table.query().select(Select::columns(&[
            COL_FILE_ID,
            COL_FILEPATH,
            COL_START_LINE,
            COL_END_LINE,
            COL_EMBEDDING,
        ]));
        if let Some(w) = where_clause {
            query = query.only_if(translate_where_to_struct(w));
        }
        let batches: Vec<RecordBatch> = query.execute().await?.try_collect().await?;

        // Best chunk per file_id.
        let mut best: std::collections::HashMap<String, SearchHit> =
            std::collections::HashMap::new();
        for batch in &batches {
            let file_ids = str_col(batch, COL_FILE_ID)?;
            let filepaths = str_col(batch, COL_FILEPATH)?;
            let start_lines = i32_col(batch, COL_START_LINE)?;
            let end_lines = i32_col(batch, COL_END_LINE)?;
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
                let v: Vec<f32> = (0..floats.len()).map(|j| floats.value(j)).collect();
                let score = cosine_similarity(&query_embedding, &v);
                let file_id = file_ids.value(i).to_string();
                let entry = best.entry(file_id).or_insert_with(|| SearchHit {
                    filename: filepaths.value(i).to_string(),
                    score: f64::NEG_INFINITY,
                    start_line: None,
                    end_line: None,
                    chunk_text: None,
                });
                if score > entry.score {
                    entry.score = score;
                    entry.filename = filepaths.value(i).to_string();
                    entry.start_line = Some(start_lines.value(i));
                    entry.end_line = Some(end_lines.value(i));
                }
            }
        }

        let mut hits: Vec<SearchHit> = best.into_values().collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit);
        Ok(hits)
    }
}

/// Cosine similarity between two equal-length vectors. Returns 0 for a
/// zero-norm input.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        (dot / (na * nb)) as f64
    }
}

/// Minimal `--where` translator for the wave-1 throwaway search: prefix bare
/// frontmatter field references with `data.` so they resolve against the
/// denormalized table's `data` Struct column (the `files_v` view that used to
/// promote them to top level is gone). Identifier chains whose first segment
/// is a reserved top-level column, or that are SQL keywords/functions, are
/// left untouched. Full alias / internal-prefix handling is wave 2.
fn translate_where_to_struct(clause: &str) -> String {
    use regex::Regex;
    // Reserved top-level columns of the denormalized table.
    const RESERVED: &[&str] = &[
        "chunk_id",
        "file_id",
        "chunk_index",
        "start_line",
        "end_line",
        "embedding",
        "filepath",
        "content_hash",
        "data",
        "built_at",
    ];
    // SQL keywords / functions that look like identifiers but aren't columns.
    const KEYWORDS: &[&str] = &[
        "AND",
        "OR",
        "NOT",
        "IN",
        "IS",
        "NULL",
        "LIKE",
        "BETWEEN",
        "TRUE",
        "FALSE",
        "DATE",
        "TIMESTAMP",
        "EXTRACT",
        "DATE_PART",
        "FROM",
        "ARRAY_HAS",
        "ARRAY_HAS_ANY",
        "ARRAY_HAS_ALL",
    ];

    // Split off single-quoted string literals so we never rewrite inside them.
    let lit = Regex::new(r"'(?:[^']|'')*'").expect("valid literal regex");
    let ident = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*")
        .expect("valid ident regex");

    let mut out = String::with_capacity(clause.len());
    let mut last = 0;
    for m in lit.find_iter(clause) {
        // Rewrite the non-literal segment before this string literal.
        out.push_str(&rewrite_idents(
            &clause[last..m.start()],
            &ident,
            RESERVED,
            KEYWORDS,
        ));
        // Copy the literal verbatim.
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&rewrite_idents(&clause[last..], &ident, RESERVED, KEYWORDS));
    out
}

fn rewrite_idents(
    segment: &str,
    ident: &regex::Regex,
    reserved: &[&str],
    keywords: &[&str],
) -> String {
    ident
        .replace_all(segment, |caps: &regex::Captures| {
            let chain = &caps[0];
            let first = chain.split('.').next().unwrap_or(chain);
            let is_reserved = reserved.contains(&first);
            let is_keyword = keywords.iter().any(|k| k.eq_ignore_ascii_case(chain));
            if is_reserved || is_keyword {
                chain.to_string()
            } else {
                format!("data.{chain}")
            }
        })
        .into_owned()
}

/// Downcast a named column to `StringArray`.
fn str_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing column {name}"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow::anyhow!("expected StringArray for {name}"))
}

/// Downcast a named column to `Int32Array`.
fn i32_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a Int32Array> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing column {name}"))?
        .as_any()
        .downcast_ref::<Int32Array>()
        .ok_or_else(|| anyhow::anyhow!("expected Int32Array for {name}"))
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
                chunk_text: String::new(),
                embedding: vec![0.9, 0.1, 0.0, 0.0],
            },
            ChunkRow {
                chunk_id: "c2".into(),
                file_id: "f2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                chunk_text: String::new(),
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
            schema_hash: "test".into(),
        }
    }

    #[tokio::test]
    async fn write_and_read_metadata() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();

        let meta = backend.read_metadata().await.unwrap();
        assert_eq!(meta, Some(test_metadata()));
    }

    #[tokio::test]
    async fn write_and_stats() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();

        let stats = backend.stats().await.unwrap().unwrap();
        assert_eq!(stats.files_indexed, 2);
        assert_eq!(stats.chunks, 2);
    }

    #[tokio::test]
    async fn exists_false_then_true() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        assert!(!backend.exists());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();

        assert!(backend.exists());
    }

    #[tokio::test]
    async fn clean_removes_index() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();
        assert!(backend.exists());

        backend.clean().await.unwrap();
        assert!(!backend.exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn embedding_dimension_correct() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        assert_eq!(backend.embedding_dimension().await.unwrap(), None);

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();

        assert_eq!(backend.embedding_dimension().await.unwrap(), Some(4));
    }

    #[tokio::test]
    async fn search_returns_hits() {
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::lance(tmp.path());

        backend
            .write_index(
                &test_schema_fields(),
                &test_files(),
                &test_chunks(),
                test_metadata(),
            )
            .await
            .unwrap();

        // Query vector close to rust.md's embedding
        let hits = backend
            .search(
                vec![1.0, 0.0, 0.0, 0.0],
                None,
                10,
                "",
                &std::collections::HashMap::new(),
            )
            .await
            .unwrap();

        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].filename, "blog/rust.md");
        assert!(hits[0].score > hits[1].score);
        assert_eq!(hits[0].start_line, Some(1));
        assert_eq!(hits[0].end_line, Some(4));
    }

    #[test]
    fn translate_where_prefixes_frontmatter_fields() {
        // bare frontmatter field gets data. prefix; keyword/literal untouched
        assert_eq!(
            translate_where_to_struct("draft = false"),
            "data.draft = false"
        );
        // dotted leaf
        assert_eq!(
            translate_where_to_struct("calibration.baseline.wavelength > 800"),
            "data.calibration.baseline.wavelength > 800"
        );
        // reserved top-level column left alone
        assert_eq!(translate_where_to_struct("file_id = 'x'"), "file_id = 'x'");
        // already data-qualified left alone; string literal untouched
        assert_eq!(
            translate_where_to_struct("data.tags = 'sci-fi' AND rating > 3"),
            "data.tags = 'sci-fi' AND data.rating > 3"
        );
    }
}
