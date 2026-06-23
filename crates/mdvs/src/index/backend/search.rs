//! Search-side method of [`LanceBackend`] plus the `--where` SQL
//! translator.
//!
//! The search method (`LanceBackend::search`) dispatches on `SearchMode`
//! across three native LanceDB paths: vector `nearest_to`, BM25
//! `full_text_search`, and hybrid (RRF reranker fusing both). The
//! `--where` clause translator below it walks the user's SQL fragment,
//! prefixes bare frontmatter field names with `data.`, rejects
//! `Array(Float)` references (lance-encoding 6.0 panics on them — see
//! TODO-0159), and leaves quoted literals untouched.

use super::{LanceBackend, SearchHit, SearchMode, f32_col, i32_col, str_col};
use crate::index::storage::{
    COL_BUILT_AT, COL_CHUNK_ID, COL_CHUNK_INDEX, COL_CHUNK_TEXT, COL_CONTENT_HASH, COL_EMBEDDING,
    COL_END_LINE, COL_FILE_ID, COL_FILEPATH, COL_START_LINE,
};
use arrow::array::RecordBatch;
use arrow::datatypes::DataType;
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::DistanceType;
use lancedb::query::{ExecutableQuery, QueryBase, Select};

/// Over-fetch multiplier for chunk→file dedupe: to surface N files we pull
/// roughly N×factor chunk-level hits, since several chunks may share a file.
const OVER_FETCH_FACTOR: usize = 3;

/// Reserved top-level columns of the denormalized Lance table.
pub(super) const RESERVED_COLS: &[&str] = &[
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
pub(super) const SQL_KEYWORDS: &[&str] = &[
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

impl LanceBackend {
    /// Native LanceDB search. `mode` selects vector (`nearest_to` + cosine),
    /// full-text (BM25 over `chunk_text`), or hybrid (both, fused by LanceDB's
    /// default RRF reranker). Over-fetches `limit * OVER_FETCH_FACTOR`
    /// chunk-level hits, then keeps the best-scoring chunk per `file_id`.
    ///
    /// `query_embedding` is required for `Semantic` and `Hybrid`; `Fulltext`
    /// runs BM25 only and ignores it. Passing `None` for a mode that needs
    /// the embedding is a programmer error.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn search(
        &self,
        query_embedding: Option<Vec<f32>>,
        query_text: &str,
        mode: SearchMode,
        where_clause: Option<&str>,
        limit: usize,
        internal_prefix: &str,
        aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<SearchResults> {
        // `--limit 0` means no results; LanceDB rejects a zero `k`, so short-
        // circuit rather than surface a cryptic "k must be positive" error.
        if limit == 0 {
            return Ok(SearchResults {
                hits: vec![],
                where_rewrites: vec![],
            });
        }
        let Some(table) = self.open_table().await? else {
            return Ok(SearchResults {
                hits: vec![],
                where_rewrites: vec![],
            });
        };

        let (translated, where_rewrites) = match where_clause {
            Some(w) => {
                let schema = table.schema().await?;
                let data_children = data_child_names(schema.as_ref());
                let float_lists = float_list_child_names(schema.as_ref());
                let array_fields = array_child_names(schema.as_ref(), &float_lists);
                let result = translate_where_to_struct(
                    w,
                    &data_children,
                    &float_lists,
                    &array_fields,
                    internal_prefix,
                    aliases,
                )?;
                (Some(result.clause), result.rewrites)
            }
            None => (None, vec![]),
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
                let embedding = query_embedding.ok_or_else(|| {
                    anyhow::anyhow!("Semantic search requires query_embedding; got None")
                })?;
                let mut q = table
                    .query()
                    .select(projection)
                    .nearest_to(embedding)?
                    .distance_type(DistanceType::Cosine)
                    .limit(k);
                if let Some(w) = &translated {
                    q = q.only_if(w);
                }
                q.execute().await?.try_collect().await?
            }
            SearchMode::Hybrid => {
                let embedding = query_embedding.ok_or_else(|| {
                    anyhow::anyhow!("Hybrid search requires query_embedding; got None")
                })?;
                let mut q = table
                    .query()
                    .select(projection)
                    .nearest_to(embedding)?
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
        Ok(SearchResults {
            hits,
            where_rewrites,
        })
    }
}

/// Search results bundled with any array-field rewrites that fired during
/// `--where` translation. Surfaced to the user as a "Note" block at the top
/// of the search output so the rewrite isn't magic.
pub struct SearchResults {
    /// File-deduped hits, ordered by descending score.
    pub hits: Vec<SearchHit>,
    /// Array-field rewrites — empty when no `--where` clause was passed or
    /// when nothing needed rewriting.
    pub where_rewrites: Vec<super::where_translator::WhereRewrite>,
}

pub(super) use super::where_translator::translate_where_to_struct;

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

/// Top-level `data` Struct children whose Arrow type is `List<T>` or
/// `LargeList<T>` for any element type *other than* float — i.e. fields
/// declared as `Array(String)`, `Array(Integer)`, `Array(Boolean)`,
/// `Array(Date)`, or `Array(DateTime)` in `mdvs.toml`. Used by the `--where`
/// translator to recognise array-field comparisons and rewrite them as
/// `array_has(...)` calls. `Array(Float)` lists are returned by
/// [`float_list_child_names`] and are rejected up front rather than
/// rewritten.
pub(super) fn array_child_names(
    schema: &arrow::datatypes::Schema,
    float_list_fields: &std::collections::HashSet<String>,
) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    if let Ok(field) = schema.field_with_name(crate::index::storage::COL_DATA)
        && let DataType::Struct(children) = field.data_type()
    {
        for child in children {
            let is_list = matches!(
                child.data_type(),
                DataType::List(_) | DataType::LargeList(_)
            );
            if is_list && !float_list_fields.contains(child.name()) {
                names.insert(child.name().clone());
            }
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
