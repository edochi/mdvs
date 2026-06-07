mod read;
mod search;

use crate::discover::field_type::FieldType;
use crate::index::storage::{
    BuildMetadata, COL_CHUNK_TEXT, COL_EMBEDDING, COL_FILE_ID, ChunkRow, FileIndexEntry, FileRow,
    build_index_batch,
};
use anyhow::Context;
use arrow::array::{
    Float32Array, Int32Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StringArray,
};
use lancedb::DistanceType;
use lancedb::connection::LanceFileVersion;
use lancedb::database::CreateTableMode;
use lancedb::database::listing::{ListingDatabaseOptions, NewTableConfig};
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Name of the single denormalized Lance table.
const LANCE_TABLE: &str = "index";

/// Minimum chunk count before building the IVF-PQ vector index. Below this,
/// exact flat search is used (IVF-PQ can't train on tiny corpora, and flat is
/// fast enough — LanceDB recommends it up to ~100k vectors).
const VECTOR_INDEX_MIN_ROWS: usize = 10_000;

/// Retrieval mode for `mdvs search` (`--mode`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// Vector similarity only (cosine over embeddings).
    Semantic,
    /// BM25 full-text only (over `chunk_text`).
    Fulltext,
    /// Vector + BM25 fused by reciprocal rank fusion (default).
    #[default]
    Hybrid,
}

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

    /// Full-rebuild write: replace the Lance table from scratch via
    /// `CreateTableMode::Overwrite`. Used on the first build and whenever
    /// `--force` is passed. For the small-delta case, use
    /// [`Backend::write_index_incremental`] instead — it avoids the full
    /// table rewrite by deleting only the changed file_ids and appending
    /// the newly embedded chunks.
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

    /// Incremental write: delete rows for the given file_ids (changed +
    /// removed), append rows for the given new chunks, refresh the schema
    /// metadata, and optimize the indexes. Used when an existing index is
    /// present and the change set is small — avoids the full-table rewrite
    /// (`CreateTableMode::Overwrite`) that `write_index` performs.
    ///
    /// `file_ids_to_clear` must contain every file_id whose existing rows
    /// must go (typically: removed files + edited files whose chunks are
    /// being replaced). It may overlap with file_ids referenced by
    /// `new_chunks` — those files first have their old rows deleted, then
    /// their new chunks added.
    #[instrument(name = "write_index_incremental", skip_all)]
    pub async fn write_index_incremental(
        &self,
        schema_fields: &[(String, FieldType)],
        file_ids_to_clear: &[String],
        files: &[FileRow],
        new_chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        match self {
            Backend::Lance(b) => {
                b.write_index_incremental(
                    schema_fields,
                    file_ids_to_clear,
                    files,
                    new_chunks,
                    metadata,
                )
                .await
            }
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
    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        query_embedding: Option<Vec<f32>>,
        query_text: &str,
        mode: SearchMode,
        where_clause: Option<&str>,
        limit: usize,
        internal_prefix: &str,
        aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        match self {
            Backend::Lance(b) => {
                b.search(
                    query_embedding,
                    query_text,
                    mode,
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
        // Write Lance v2.2 files. The crate default is v2.1, whose miniblock
        // encoding caps a chunk at 32 KiB (u16 metadata). Sparse nested
        // frontmatter (many optional fields, mostly null per row) generates
        // dense rep/def levels that Lance's heuristic mis-routes into
        // miniblock, overflowing 32 KiB and panicking during build on large
        // corpora. v2.2 uses u32 metadata (4 GiB cap), sidestepping it. Only
        // affects newly created tables; reads of either version work.
        let db_options = ListingDatabaseOptions {
            new_table_config: NewTableConfig {
                data_storage_version: Some(LanceFileVersion::V2_2),
                ..Default::default()
            },
            ..Default::default()
        };
        lancedb::connect(&uri)
            .database_options(&db_options)
            .execute()
            .await
            .context("connecting to Lance database")
    }

    /// Open the index table, or `None` if it doesn't exist yet.
    pub(super) async fn open_table(&self) -> anyhow::Result<Option<lancedb::table::Table>> {
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
        let table = conn
            .create_table(LANCE_TABLE, reader)
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await
            .context("creating Lance index table")?;

        self.build_indexes(&table, chunks.len()).await?;
        Ok(())
    }

    async fn write_index_incremental(
        &self,
        schema_fields: &[(String, FieldType)],
        file_ids_to_clear: &[String],
        files: &[FileRow],
        new_chunks: &[ChunkRow],
        metadata: BuildMetadata,
    ) -> anyhow::Result<()> {
        let conn = self.connect().await?;
        let table = conn
            .open_table(LANCE_TABLE)
            .execute()
            .await
            .context("opening Lance index table for incremental write")?;

        // 1. Delete rows for changed + removed files. SQL IN clause.
        //    file_ids are UUIDs (no quotes/specials), so direct
        //    interpolation is safe.
        if !file_ids_to_clear.is_empty() {
            let in_list = file_ids_to_clear
                .iter()
                .map(|id| format!("'{id}'"))
                .collect::<Vec<_>>()
                .join(", ");
            let predicate = format!("{COL_FILE_ID} IN ({in_list})");
            table
                .delete(&predicate)
                .await
                .context("deleting rows for changed/removed files")?;
        }

        // 2. Append rows for the new chunks. `build_index_batch` joins
        //    each chunk with its parent file_row to produce the
        //    denormalized shape the table expects. `files` may include
        //    file_rows for files with no new chunks — those are simply
        //    ignored by the join.
        if !new_chunks.is_empty() {
            let batch = build_index_batch(schema_fields, files, new_chunks)?;
            let schema = batch.schema();
            let reader: Box<dyn RecordBatchReader + Send> =
                Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));
            table
                .add(reader)
                .execute()
                .await
                .context("appending new chunks")?;
        }

        // 3. Refresh the schema-level BuildMetadata (`mdvs.*` keys).
        //    `replace_schema_metadata` lives on `NativeTable` only; the
        //    local-file backend always returns `Some(_)`.
        let native = table.as_native().context(
            "incremental write requires a NativeTable (local file backend); \
             this should never happen for an existing Lance dataset",
        )?;
        native
            .replace_schema_metadata(metadata.to_hash_map())
            .await
            .context("updating schema metadata")?;

        // 4. Optimize: compacts new fragments and updates the FTS +
        //    vector indexes so the just-added rows participate in
        //    indexed lookups (until then Lance falls back to scanning
        //    the un-indexed delta — correct but slower).
        table
            .optimize(lancedb::table::OptimizeAction::All)
            .await
            .context("optimizing after incremental write")?;

        Ok(())
    }

    /// Build the FTS index always, and the cosine IVF-PQ vector index only
    /// above [`VECTOR_INDEX_MIN_ROWS`] (IVF-PQ needs enough vectors to train;
    /// below the threshold LanceDB's exact flat scan is used instead).
    async fn build_indexes(
        &self,
        table: &lancedb::table::Table,
        n_chunks: usize,
    ) -> anyhow::Result<()> {
        if n_chunks == 0 {
            return Ok(());
        }
        table
            .create_index(&[COL_CHUNK_TEXT], Index::FTS(Default::default()))
            .execute()
            .await
            .context("building full-text index")?;

        if n_chunks >= VECTOR_INDEX_MIN_ROWS {
            table
                .create_index(
                    &[COL_EMBEDDING],
                    Index::IvfPq(IvfPqIndexBuilder::default().distance_type(DistanceType::Cosine)),
                )
                .execute()
                .await
                .context("building vector index")?;
        }
        Ok(())
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
}

// ============================================================================
// Column-downcast helpers, shared by the read/search sub-modules.
// ============================================================================

/// Downcast a named column to `StringArray`.
pub(super) fn str_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a StringArray> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing column {name}"))?
        .as_any()
        .downcast_ref::<StringArray>()
        .ok_or_else(|| anyhow::anyhow!("expected StringArray for {name}"))
}

/// Downcast a named column to `Int32Array`.
pub(super) fn i32_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a Int32Array> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing column {name}"))?
        .as_any()
        .downcast_ref::<Int32Array>()
        .ok_or_else(|| anyhow::anyhow!("expected Int32Array for {name}"))
}

/// Downcast a named column to `Float32Array`.
pub(super) fn f32_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a Float32Array> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing score column {name}"))?
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| anyhow::anyhow!("expected Float32Array for {name}"))
}

#[cfg(test)]
mod tests {
    use super::search::translate_where_to_struct;
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
                dim: None,
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
                Some(vec![1.0, 0.0, 0.0, 0.0]),
                "rust",
                SearchMode::Semantic,
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

    fn fields(names: &[&str]) -> std::collections::HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn xlate(clause: &str, children: &[&str]) -> String {
        translate_where_to_struct(
            clause,
            &fields(children),
            &std::collections::HashSet::new(),
            "",
            &std::collections::HashMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn translate_where_prefixes_frontmatter_fields() {
        // bare frontmatter field gets data. prefix; keyword/literal untouched
        assert_eq!(xlate("draft = false", &["draft"]), "data.draft = false");
        // dotted leaf (first segment is the frontmatter field)
        assert_eq!(
            xlate("calibration.baseline.wavelength > 800", &["calibration"]),
            "data.calibration.baseline.wavelength > 800"
        );
        // genuine internal column (not a frontmatter field) left alone
        assert_eq!(xlate("file_id = 'x'", &["draft"]), "file_id = 'x'");
        // already data-qualified left alone; string literal untouched
        assert_eq!(
            xlate("data.tags = 'sci-fi' AND rating > 3", &["tags", "rating"]),
            "data.tags = 'sci-fi' AND data.rating > 3"
        );
    }

    #[test]
    fn translate_where_collision_errors_without_aliasing() {
        // frontmatter field named like an internal column, no aliasing → error
        let err = translate_where_to_struct(
            "file_id = 'x'",
            &fields(&["file_id"]),
            &std::collections::HashSet::new(),
            "",
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("ambiguous column 'file_id'"));
    }

    #[test]
    fn translate_where_collision_resolved_by_prefix() {
        // with internal_prefix set, bare name = frontmatter field; the prefixed
        // form addresses the internal column.
        let out = translate_where_to_struct(
            "file_id = 'x' AND _file_id = 'y'",
            &fields(&["file_id"]),
            &std::collections::HashSet::new(),
            "_",
            &std::collections::HashMap::new(),
        )
        .unwrap();
        assert_eq!(out, "data.file_id = 'x' AND file_id = 'y'");
    }

    // --- Translator: comparison operators ---

    #[test]
    fn translate_where_eq() {
        assert_eq!(
            xlate("status = 'active'", &["status"]),
            "data.status = 'active'"
        );
    }

    #[test]
    fn translate_where_not_equal_both_spellings() {
        assert_eq!(xlate("rating <> 5", &["rating"]), "data.rating <> 5");
        assert_eq!(xlate("rating != 5", &["rating"]), "data.rating != 5");
    }

    #[test]
    fn translate_where_lt_gt_le_ge() {
        assert_eq!(xlate("rating < 3", &["rating"]), "data.rating < 3");
        assert_eq!(xlate("rating > 3", &["rating"]), "data.rating > 3");
        assert_eq!(xlate("rating <= 3", &["rating"]), "data.rating <= 3");
        assert_eq!(xlate("rating >= 3", &["rating"]), "data.rating >= 3");
    }

    #[test]
    fn translate_where_in_list() {
        assert_eq!(
            xlate("rating IN (4, 5)", &["rating"]),
            "data.rating IN (4, 5)"
        );
    }

    #[test]
    fn translate_where_in_string_list_literals_untouched() {
        // string literals — even ones that look like field names — are not rewritten
        assert_eq!(
            xlate("status IN ('active', 'rating')", &["status", "rating"]),
            "data.status IN ('active', 'rating')"
        );
    }

    #[test]
    fn translate_where_between() {
        assert_eq!(
            xlate("rating BETWEEN 3 AND 5", &["rating"]),
            "data.rating BETWEEN 3 AND 5"
        );
    }

    #[test]
    fn translate_where_like() {
        assert_eq!(
            xlate("title LIKE 'Rust%'", &["title"]),
            "data.title LIKE 'Rust%'"
        );
    }

    #[test]
    fn translate_where_is_null_and_not_null() {
        assert_eq!(xlate("note IS NULL", &["note"]), "data.note IS NULL");
        assert_eq!(
            xlate("note IS NOT NULL", &["note"]),
            "data.note IS NOT NULL"
        );
    }

    // --- Translator: keywords / functions / literals ---

    #[test]
    fn translate_where_keywords_case_insensitive() {
        assert_eq!(
            xlate("a > 1 and b < 2 Or c = 3", &["a", "b", "c"]),
            "data.a > 1 and data.b < 2 Or data.c = 3"
        );
    }

    #[test]
    fn translate_where_boolean_literals_untouched() {
        assert_eq!(xlate("draft = true", &["draft"]), "data.draft = true");
        assert_eq!(xlate("draft = FALSE", &["draft"]), "data.draft = FALSE");
    }

    #[test]
    fn translate_where_numbers_untouched() {
        assert_eq!(
            xlate("wavelength = 850.5", &["wavelength"]),
            "data.wavelength = 850.5"
        );
    }

    #[test]
    fn translate_where_date_literal() {
        assert_eq!(
            xlate("published >= date '2024-01-01'", &["published"]),
            "data.published >= date '2024-01-01'"
        );
    }

    #[test]
    fn translate_where_date_as_field_name() {
        // A frontmatter field literally named `date` must be prefixed, while
        // the `date '...'` literal keyword on the right is left alone.
        assert_eq!(
            xlate("date >= date '2032-01-01'", &["date"]),
            "data.date >= date '2032-01-01'"
        );
    }

    #[test]
    fn translate_where_timestamp_as_field_name() {
        assert_eq!(
            xlate(
                "timestamp < timestamp '2024-01-01T00:00:00Z'",
                &["timestamp"]
            ),
            "data.timestamp < timestamp '2024-01-01T00:00:00Z'"
        );
    }

    #[test]
    fn translate_where_timestamp_literal() {
        assert_eq!(
            xlate(
                "synced_at < timestamp '2024-01-01T00:00:00Z'",
                &["synced_at"]
            ),
            "data.synced_at < timestamp '2024-01-01T00:00:00Z'"
        );
    }

    #[test]
    fn translate_where_array_has() {
        // array_has is a function (keyword) → untouched; its column arg → data.
        assert_eq!(
            xlate("array_has(tags, 'rust')", &["tags"]),
            "array_has(data.tags, 'rust')"
        );
    }

    #[test]
    fn translate_where_scalar_functions_left_unprefixed() {
        // A bare identifier followed by `(` is a function name, not a column.
        assert_eq!(
            xlate("lower(status) = 'active'", &["status"]),
            "lower(data.status) = 'active'"
        );
        assert_eq!(
            xlate("length(title) > 10", &["title"]),
            "length(data.title) > 10"
        );
        assert_eq!(
            xlate("abs(drift_rate) < 1", &["drift_rate"]),
            "abs(data.drift_rate) < 1"
        );
        // function with whitespace before the paren
        assert_eq!(
            xlate("upper (status) = 'A'", &["status"]),
            "upper (data.status) = 'A'"
        );
    }

    #[test]
    fn translate_where_field_not_followed_by_paren_is_prefixed() {
        // a field literally named like a function, used as a column, IS prefixed
        assert_eq!(xlate("lower > 5", &["lower"]), "data.lower > 5");
    }

    #[test]
    fn translate_where_arithmetic_on_column() {
        assert_eq!(
            xlate("sample_count + 1 > 5", &["sample_count"]),
            "data.sample_count + 1 > 5"
        );
    }

    #[test]
    fn translate_where_string_literal_with_field_like_token() {
        // a literal containing a token that matches a field name is left intact
        assert_eq!(
            xlate("title = 'draft status'", &["title", "draft", "status"]),
            "data.title = 'draft status'"
        );
    }

    #[test]
    fn translate_where_escaped_quote_in_literal() {
        assert_eq!(
            xlate("author = 'O''Brien'", &["author"]),
            "data.author = 'O''Brien'"
        );
    }

    #[test]
    fn translate_where_literal_with_dotted_token() {
        // a dotted token inside a literal must not be prefixed
        assert_eq!(xlate("path = 'a.b.c'", &["path"]), "data.path = 'a.b.c'");
    }

    // --- Translator: struct paths & internal columns ---

    #[test]
    fn translate_where_all_internal_columns_left_alone() {
        for col in [
            "chunk_id",
            "file_id",
            "chunk_index",
            "start_line",
            "end_line",
            "chunk_text",
            "embedding",
            "filepath",
            "content_hash",
            "built_at",
        ] {
            // not a frontmatter field → internal column, left as-is
            let clause = format!("{col} = 1");
            assert_eq!(
                xlate(&clause, &["other"]),
                clause,
                "{col} should be left alone"
            );
        }
    }

    #[test]
    fn translate_where_filepath_filter() {
        assert_eq!(
            xlate("filepath LIKE 'blog/%'", &["title"]),
            "filepath LIKE 'blog/%'"
        );
    }

    #[test]
    fn translate_where_deeply_nested_dotted() {
        assert_eq!(xlate("a.b.c.d.e = 1", &["a"]), "data.a.b.c.d.e = 1");
    }

    #[test]
    fn translate_where_already_data_qualified_untouched() {
        // first segment `data` is reserved → left as-is (idempotent)
        assert_eq!(xlate("data.title = 'x'", &["title"]), "data.title = 'x'");
    }

    #[test]
    fn translate_where_unknown_field_treated_as_frontmatter() {
        // a name that's neither reserved nor a known field is assumed frontmatter
        assert_eq!(xlate("typo = 1", &["title"]), "data.typo = 1");
    }

    #[test]
    fn translate_where_field_with_digits_and_underscores() {
        assert_eq!(
            xlate("sample_count_2 > 10", &["sample_count_2"]),
            "data.sample_count_2 > 10"
        );
    }

    #[test]
    fn translate_where_none_when_no_clause_is_handled_by_caller() {
        // empty clause translates to empty (caller passes None for absent --where)
        assert_eq!(xlate("", &["title"]), "");
    }

    // --- Translator: collisions & aliasing ---

    #[test]
    fn translate_where_collision_resolved_by_alias() {
        let mut aliases = std::collections::HashMap::new();
        aliases.insert("file_id".to_string(), "fid".to_string());
        let out = translate_where_to_struct(
            "file_id = 'x' AND fid = 'y'",
            &fields(&["file_id"]),
            &std::collections::HashSet::new(),
            "",
            &aliases,
        )
        .unwrap();
        // bare `file_id` = frontmatter field → data.file_id; alias `fid` → internal file_id
        assert_eq!(out, "data.file_id = 'x' AND file_id = 'y'");
    }

    #[test]
    fn translate_where_no_collision_when_field_not_reserved() {
        // a frontmatter field that doesn't shadow an internal column never collides
        assert_eq!(xlate("author = 'x'", &["author"]), "data.author = 'x'");
    }

    #[test]
    fn translate_where_array_float_field_is_rejected() {
        // Filtering on an Array(Float) field would panic inside Lance — we
        // refuse it up front with a clear message instead.
        let float_lists: std::collections::HashSet<String> =
            ["measurement_values".to_string()].into_iter().collect();
        for clause in [
            "measurement_values IS NOT NULL",
            "array_has(measurement_values, 0.5)",
            "data.measurement_values IS NULL",
        ] {
            let err = translate_where_to_struct(
                clause,
                &fields(&["measurement_values"]),
                &float_lists,
                "",
                &std::collections::HashMap::new(),
            )
            .unwrap_err();
            assert!(
                err.to_string()
                    .contains("Array(Float) field 'measurement_values'"),
                "clause `{clause}` should produce the Array(Float) error, got: {err}"
            );
        }
    }

    #[test]
    fn translate_where_collision_only_on_matching_reserved_name() {
        // `filepath` frontmatter field collides; `title` does not — mixed clause
        let err = translate_where_to_struct(
            "filepath = 'a' AND title = 'b'",
            &fields(&["filepath", "title"]),
            &std::collections::HashSet::new(),
            "",
            &std::collections::HashMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("ambiguous column 'filepath'"));
    }
}
