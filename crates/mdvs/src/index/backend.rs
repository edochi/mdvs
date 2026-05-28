use crate::discover::field_type::FieldType;
use crate::index::storage::{
    BuildMetadata, COL_BUILT_AT, COL_CHUNK_ID, COL_CHUNK_INDEX, COL_CHUNK_TEXT, COL_CONTENT_HASH,
    COL_EMBEDDING, COL_END_LINE, COL_FILE_ID, COL_FILEPATH, COL_START_LINE, ChunkRow,
    FileIndexEntry, FileRow, build_index_batch,
};
use anyhow::Context;
use arrow::array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow::datatypes::DataType;
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::DistanceType;
use lancedb::connection::LanceFileVersion;
use lancedb::database::CreateTableMode;
use lancedb::database::listing::{ListingDatabaseOptions, NewTableConfig};
use lancedb::index::Index;
use lancedb::index::vector::IvfPqIndexBuilder;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use serde::Serialize;
use std::collections::HashSet;
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
    #[allow(clippy::too_many_arguments)]
    pub async fn search(
        &self,
        query_embedding: Vec<f32>,
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
        let table = conn
            .create_table(LANCE_TABLE, reader)
            .mode(CreateTableMode::Overwrite)
            .execute()
            .await
            .context("creating Lance index table")?;

        self.build_indexes(&table, chunks.len()).await?;
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

    /// Native LanceDB search. `mode` selects vector (`nearest_to` + cosine),
    /// full-text (BM25 over `chunk_text`), or hybrid (both, fused by LanceDB's
    /// default RRF reranker). Over-fetches `limit * OVER_FETCH_FACTOR`
    /// chunk-level hits, then keeps the best-scoring chunk per `file_id`.
    #[allow(clippy::too_many_arguments)]
    async fn search(
        &self,
        query_embedding: Vec<f32>,
        query_text: &str,
        mode: SearchMode,
        where_clause: Option<&str>,
        limit: usize,
        internal_prefix: &str,
        aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Vec<SearchHit>> {
        // `--limit 0` means no results; LanceDB rejects a zero `k`, so short-
        // circuit rather than surface a cryptic "k must be positive" error.
        if limit == 0 {
            return Ok(vec![]);
        }
        let Some(table) = self.open_table().await? else {
            return Ok(vec![]);
        };

        let translated = match where_clause {
            Some(w) => {
                let schema = table.schema().await?;
                let data_children = data_child_names(schema.as_ref());
                let float_lists = float_list_child_names(schema.as_ref());
                Some(translate_where_to_struct(
                    w,
                    &data_children,
                    &float_lists,
                    internal_prefix,
                    aliases,
                )?)
            }
            None => None,
        };
        let k = limit.saturating_mul(OVER_FETCH_FACTOR);
        let fts = || FullTextSearchQuery::new(query_text.to_string());

        // Project only the columns we need. Critically, this excludes the
        // `data` Struct and `embedding` columns — fetching them via the
        // post-vector-search "take" trips a buffer-slicing panic in Lance's
        // encoder (lance-encoding 6.0), and we don't need them for results.
        let projection = Select::columns(&[
            COL_FILE_ID,
            COL_FILEPATH,
            COL_START_LINE,
            COL_END_LINE,
            COL_CHUNK_TEXT,
        ]);

        // The branches have distinct query types (VectorQuery vs Query), so
        // each collects its own batches.
        let batches: Vec<RecordBatch> = match mode {
            SearchMode::Semantic => {
                let mut q = table
                    .query()
                    .select(projection)
                    .nearest_to(query_embedding)?
                    .distance_type(DistanceType::Cosine)
                    .limit(k);
                if let Some(w) = &translated {
                    q = q.only_if(w);
                }
                q.execute().await?.try_collect().await?
            }
            SearchMode::Hybrid => {
                let mut q = table
                    .query()
                    .select(projection)
                    .nearest_to(query_embedding)?
                    .distance_type(DistanceType::Cosine)
                    .full_text_search(fts())
                    .limit(k);
                if let Some(w) = &translated {
                    q = q.only_if(w);
                }
                q.execute().await?.try_collect().await?
            }
            SearchMode::Fulltext => {
                let mut q = table
                    .query()
                    .select(projection)
                    .full_text_search(fts())
                    .limit(k);
                if let Some(w) = &translated {
                    q = q.only_if(w);
                }
                q.execute().await?.try_collect().await?
            }
        };

        // Per-mode score column. Semantic returns cosine *distance* (lower is
        // better → similarity = 1 - distance); the others return a score where
        // higher is better.
        let score_col = match mode {
            SearchMode::Semantic => "_distance",
            SearchMode::Fulltext => "_score",
            SearchMode::Hybrid => "_relevance_score",
        };

        let mut best: std::collections::HashMap<String, SearchHit> =
            std::collections::HashMap::new();
        for batch in &batches {
            // A zero-result hybrid query returns an empty batch whose schema
            // omits the projected columns; skip it rather than fail the lookup.
            if batch.num_rows() == 0 {
                continue;
            }
            let file_ids = str_col(batch, COL_FILE_ID)?;
            let filepaths = str_col(batch, COL_FILEPATH)?;
            let start_lines = i32_col(batch, COL_START_LINE)?;
            let end_lines = i32_col(batch, COL_END_LINE)?;
            let chunk_texts = str_col(batch, COL_CHUNK_TEXT)?;
            let scores = f32_col(batch, score_col)?;
            for i in 0..batch.num_rows() {
                let raw = scores.value(i) as f64;
                let score = if mode == SearchMode::Semantic {
                    1.0 - raw
                } else {
                    raw
                };
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
                    entry.chunk_text = Some(chunk_texts.value(i).to_string());
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

/// Over-fetch multiplier for chunk→file dedupe: to surface N files we pull
/// roughly N×factor chunk-level hits, since several chunks may share a file.
const OVER_FETCH_FACTOR: usize = 3;

/// Reserved top-level columns of the denormalized Lance table.
const RESERVED_COLS: &[&str] = &[
    COL_CHUNK_ID,
    COL_FILE_ID,
    COL_CHUNK_INDEX,
    COL_START_LINE,
    COL_END_LINE,
    COL_CHUNK_TEXT,
    COL_EMBEDDING,
    COL_FILEPATH,
    COL_CONTENT_HASH,
    "data",
    COL_BUILT_AT,
];

/// SQL keywords / functions that look like identifiers but aren't columns.
/// `DATE`/`TIMESTAMP` are intentionally absent: they're keywords only when
/// introducing a literal (`date '...'`), which is handled contextually, so a
/// plain frontmatter field named `date` still resolves correctly.
const SQL_KEYWORDS: &[&str] = &[
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
    "EXTRACT",
    "DATE_PART",
    "FROM",
    "ARRAY_HAS",
    "ARRAY_HAS_ANY",
    "ARRAY_HAS_ALL",
];

/// Schema-aware `--where` translator. Frontmatter fields (children of the
/// `data` Struct) are prefixed with `data.`; genuine internal columns are left
/// top-level. A name that is *both* a frontmatter field and an internal column
/// is a collision: it's resolved toward the frontmatter field when
/// `internal_prefix`/aliases give the internal column another name, otherwise
/// it errors (mirroring the old engine). Single-quoted string literals are
/// never rewritten.
fn translate_where_to_struct(
    clause: &str,
    data_children: &std::collections::HashSet<String>,
    float_list_fields: &std::collections::HashSet<String>,
    internal_prefix: &str,
    aliases: &std::collections::HashMap<String, String>,
) -> anyhow::Result<String> {
    use regex::Regex;

    // Reverse alias lookup: alias name -> real internal column.
    let alias_to_internal: std::collections::HashMap<&str, &str> = aliases
        .iter()
        .map(|(col, alias)| (alias.as_str(), col.as_str()))
        .collect();
    let has_aliasing = !internal_prefix.is_empty() || !aliases.is_empty();

    // A string literal, optionally preceded by a `date`/`timestamp` keyword so
    // the whole `date '...'` literal is protected as one unit (otherwise a
    // frontmatter field named `date` and the `date` literal keyword are
    // indistinguishable once the literal is split off).
    let lit =
        Regex::new(r"(?i)(?:\b(?:date|timestamp)\s+)?'(?:[^']|'')*'").expect("valid literal regex");
    let ident = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*")
        .expect("valid ident regex");

    let rewrite_segment = |segment: &str| -> anyhow::Result<String> {
        let mut out = String::new();
        let mut last = 0;
        for m in ident.find_iter(segment) {
            out.push_str(&segment[last..m.start()]);
            last = m.end();
            let chain = m.as_str();
            let first = chain.split('.').next().unwrap_or(chain);

            // A bare identifier immediately followed by `(` is a function call
            // (lower, length, abs, cast, coalesce, array_length, ...); leave the
            // function name and let its column arguments be rewritten normally.
            if !chain.contains('.') && segment[last..].trim_start().starts_with('(') {
                out.push_str(chain);
                continue;
            }

            // Keyword / function — leave untouched.
            if SQL_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(chain)) {
                out.push_str(chain);
                continue;
            }
            // Explicit reference to an internal column via its alias or prefix.
            if let Some(internal) = alias_to_internal.get(first) {
                out.push_str(internal);
                continue;
            }
            if !internal_prefix.is_empty()
                && let Some(stripped) = first.strip_prefix(internal_prefix)
                && RESERVED_COLS.contains(&stripped)
            {
                out.push_str(stripped);
                continue;
            }

            // Reject filters on Array(Float) fields up front — Lance panics on
            // them (TODO-0159). The referenced field is `first`, or the segment
            // after `data.` when the user pre-qualified.
            let fm_name = if first == "data" {
                chain.split('.').nth(1)
            } else {
                Some(first)
            };
            if let Some(name) = fm_name
                && float_list_fields.contains(name)
            {
                return Err(anyhow::anyhow!(
                    "filtering on Array(Float) field '{name}' is not supported in --where. \
                     Filter on a different field or store the values in a parallel scalar field."
                ));
            }

            let is_reserved = RESERVED_COLS.contains(&first);
            let is_frontmatter = data_children.contains(first);
            let rewritten = if is_reserved && is_frontmatter {
                if has_aliasing {
                    format!("data.{chain}")
                } else {
                    return Err(anyhow::anyhow!(
                        "ambiguous column '{first}' in --where: it is both a frontmatter field and \
                         an internal column. Disambiguate by setting [search].internal_prefix \
                         (e.g. \"_\") or [search.aliases].{first} = \"<alias>\""
                    ));
                }
            } else if is_reserved {
                chain.to_string()
            } else {
                format!("data.{chain}")
            };
            out.push_str(&rewritten);
        }
        out.push_str(&segment[last..]);
        Ok(out)
    };

    let mut out = String::with_capacity(clause.len());
    let mut last = 0;
    for m in lit.find_iter(clause) {
        out.push_str(&rewrite_segment(&clause[last..m.start()])?);
        out.push_str(m.as_str());
        last = m.end();
    }
    out.push_str(&rewrite_segment(&clause[last..])?);
    Ok(out)
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

/// Downcast a named column to `Float32Array`.
fn f32_col<'a>(batch: &'a RecordBatch, name: &str) -> anyhow::Result<&'a Float32Array> {
    batch
        .column_by_name(name)
        .ok_or_else(|| anyhow::anyhow!("missing score column {name}"))?
        .as_any()
        .downcast_ref::<Float32Array>()
        .ok_or_else(|| anyhow::anyhow!("expected Float32Array for {name}"))
}

/// First-level child field names of the `data` Struct column — i.e. the
/// top-level frontmatter field names. Used by the `--where` translator to
/// tell frontmatter fields from internal columns.
fn data_child_names(schema: &arrow::datatypes::Schema) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    if let Ok(field) = schema.field_with_name(crate::index::storage::COL_DATA)
        && let DataType::Struct(children) = field.data_type()
    {
        for child in children {
            names.insert(child.name().clone());
        }
    }
    names
}

/// Top-level `data` Struct children that are lists of floats (`Array(Float)`,
/// Arrow `List<Float*>`). Filtering on these via `--where` panics inside
/// lance-encoding 6.0 (TODO-0159), so the translator rejects such references
/// with a clean error instead of letting the search crash/hang.
fn float_list_child_names(schema: &arrow::datatypes::Schema) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    if let Ok(field) = schema.field_with_name(crate::index::storage::COL_DATA)
        && let DataType::Struct(children) = field.data_type()
    {
        for child in children {
            if let DataType::List(item) | DataType::LargeList(item) = child.data_type()
                && matches!(
                    item.data_type(),
                    DataType::Float16 | DataType::Float32 | DataType::Float64
                )
            {
                names.insert(child.name().clone());
            }
        }
    }
    names
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
