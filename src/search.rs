use crate::index::storage::{COL_DATA, FILES_INTERNAL_COLUMNS, resolve_view_name};
use datafusion::arrow::array::{Array, ArrayRef, FixedSizeListArray, Float32Array, Float64Array};
use datafusion::arrow::datatypes::{DataType, Field};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::execution::context::SessionContext;
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::Arc;
use tracing::instrument;

// ============================================================================
// Cosine similarity UDF — captures query vector at creation time
// ============================================================================

#[derive(Debug)]
struct CosineSimilarityUDF {
    signature: Signature,
    query_vector: Vec<f32>,
}

impl CosineSimilarityUDF {
    fn new(query_vector: Vec<f32>) -> Self {
        let dimension = query_vector.len() as i32;
        Self {
            signature: Signature::exact(
                vec![DataType::FixedSizeList(
                    Arc::new(Field::new("item", DataType::Float32, false)),
                    dimension,
                )],
                Volatility::Immutable,
            ),
            query_vector,
        }
    }
}

impl PartialEq for CosineSimilarityUDF {
    fn eq(&self, other: &Self) -> bool {
        self.query_vector.len() == other.query_vector.len()
            && self
                .query_vector
                .iter()
                .zip(&other.query_vector)
                .all(|(a, b)| a.to_bits() == b.to_bits())
    }
}

impl Eq for CosineSimilarityUDF {}

impl Hash for CosineSimilarityUDF {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for v in &self.query_vector {
            v.to_bits().hash(state);
        }
    }
}

impl ScalarUDFImpl for CosineSimilarityUDF {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn name(&self) -> &str {
        "cosine_similarity"
    }

    fn signature(&self) -> &Signature {
        &self.signature
    }

    fn return_type(&self, _args: &[DataType]) -> datafusion::common::Result<DataType> {
        Ok(DataType::Float64)
    }

    fn invoke_with_args(
        &self,
        args: ScalarFunctionArgs,
    ) -> datafusion::common::Result<ColumnarValue> {
        let args = ColumnarValue::values_to_arrays(&args.args)?;
        let embeddings = args[0]
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Internal(
                    "expected FixedSizeList<Float32>".into(),
                )
            })?;

        let dim = self.query_vector.len();

        // Pre-compute query norm
        let query_norm: f32 = self.query_vector.iter().map(|x| x * x).sum::<f32>().sqrt();

        let results: Float64Array = (0..embeddings.len())
            .map(|i| {
                if embeddings.is_null(i) {
                    return None;
                }
                let row = embeddings.value(i);
                // Infallible: we wrote the embedding column as FixedSizeList<Float32>,
                // so the inner array is always Float32Array.
                let floats = row.as_any().downcast_ref::<Float32Array>().unwrap();

                let mut dot = 0.0f32;
                let mut row_norm = 0.0f32;
                for j in 0..dim {
                    let v = floats.value(j);
                    dot += v * self.query_vector[j];
                    row_norm += v * v;
                }
                let row_norm = row_norm.sqrt();

                if query_norm == 0.0 || row_norm == 0.0 {
                    Some(0.0f64)
                } else {
                    Some((dot / (query_norm * row_norm)) as f64)
                }
            })
            .collect();

        let result_array: ArrayRef = Arc::new(results);
        Ok(ColumnarValue::from(result_array))
    }
}

// ============================================================================
// SearchContext — registered tables + UDF
// ============================================================================

/// DataFusion session with registered Parquet tables and a cosine similarity UDF.
pub struct SearchContext {
    ctx: SessionContext,
}

impl SearchContext {
    /// Register Parquet files as tables, create a view that promotes frontmatter
    /// fields to top-level columns, and bind the query embedding into the UDF.
    #[instrument(name = "register_tables", skip_all, level = "debug")]
    pub async fn new(
        files_path: &Path,
        chunks_path: &Path,
        query_embedding: Vec<f32>,
        internal_prefix: &str,
        aliases: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<Self> {
        let ctx = SessionContext::new();
        let files_str = files_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path: {}", files_path.display()))?;
        let chunks_str = chunks_path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-UTF-8 path: {}", chunks_path.display()))?;
        ctx.register_parquet("files", files_str, Default::default())
            .await?;
        ctx.register_parquet("chunks", chunks_str, Default::default())
            .await?;

        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_embedding));
        ctx.register_udf(udf);

        // Create a view that:
        // 1. Aliases internal columns (applying prefix/aliases to avoid collisions)
        // 2. Promotes data Struct children to top-level columns for bare --where names
        let files_table = ctx.table("files").await?;
        let schema = files_table.schema();

        // Collect frontmatter field names from the data Struct
        let mut frontmatter_names: Vec<String> = Vec::new();
        let mut frontmatter_projections = Vec::new();
        for field in schema.fields() {
            if field.name() == COL_DATA
                && let DataType::Struct(children) = field.data_type()
            {
                for child in children {
                    frontmatter_names.push(child.name().clone());
                    let escaped_accessor = child.name().replace('\'', "''");
                    let escaped_alias = child.name().replace('"', "\"\"");
                    frontmatter_projections.push(format!(
                        "{COL_DATA}['{escaped_accessor}'] AS \"{escaped_alias}\"",
                    ));
                }
            }
        }

        // Resolve view names for internal columns: alias > prefix > raw name
        // Then check for collisions with frontmatter fields
        let has_aliasing = !internal_prefix.is_empty() || !aliases.is_empty();
        let mut column_projections = Vec::new();

        for &col_name in FILES_INTERNAL_COLUMNS {
            let view_name = resolve_view_name(col_name, internal_prefix, aliases);

            if frontmatter_names.contains(&view_name) {
                anyhow::bail!(
                    "frontmatter field '{vn}' collides with internal column '{cn}' — resolve by:\n  \
                     - setting [search].internal_prefix (e.g., \"_\") to prefix all internal columns\n  \
                     - adding [search.aliases].{cn} = \"<alias>\" to rename just this column\n  \
                     - renaming the frontmatter field",
                    vn = view_name,
                    cn = col_name,
                );
            }

            if has_aliasing {
                column_projections.push(format!("{col_name} AS \"{view_name}\""));
            }
        }

        // When aliasing internal columns, we must build an explicit column list
        // (SELECT * would include both raw and aliased names, causing ambiguity).
        // When no aliasing, SELECT * is fine.
        let view_sql = if has_aliasing {
            // Explicit: aliased internal columns + data column + frontmatter promotions
            column_projections.push(COL_DATA.to_string());
            column_projections.extend(frontmatter_projections);
            format!(
                "CREATE VIEW files_v AS SELECT {} FROM files",
                column_projections.join(", ")
            )
        } else {
            // Simple: SELECT * + frontmatter promotions
            let extra = if frontmatter_projections.is_empty() {
                String::new()
            } else {
                format!(", {}", frontmatter_projections.join(", "))
            };
            format!("CREATE VIEW files_v AS SELECT *{extra} FROM files")
        };
        ctx.sql(&view_sql).await?;

        Ok(Self { ctx })
    }

    /// Execute a SQL query against the registered tables and return result batches.
    #[instrument(name = "query", skip_all, level = "debug")]
    pub async fn query(&self, sql: &str) -> anyhow::Result<Vec<RecordBatch>> {
        let df = self.ctx.sql(sql).await?;
        let batches = df.collect().await?;
        Ok(batches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::field_type::FieldType;
    use crate::index::storage::{
        ChunkRow, FileRow, build_chunks_batch, build_files_batch, write_parquet,
    };
    use datafusion::arrow::array::{Array, Float64Array, Int64Array, StringArray, StringViewArray};

    fn test_files() -> (Vec<(String, FieldType)>, Vec<FileRow>) {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("draft".into(), FieldType::Boolean),
        ];
        let files = vec![
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
            FileRow {
                file_id: "f3".into(),
                filename: "recipes/cooking.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Cooking Tips", "draft": true})),
                content_hash: "h3".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];
        (schema_fields, files)
    }

    #[rustfmt::skip]
    fn test_chunks() -> Vec<ChunkRow> {
        vec![
            ChunkRow { chunk_id: "c1".into(), file_id: "f1".into(), chunk_index: 0, start_line: 1, end_line: 4, embedding: vec![0.9, 0.1, 0.0, 0.0] },
            ChunkRow { chunk_id: "c2".into(), file_id: "f1".into(), chunk_index: 1, start_line: 5, end_line: 8, embedding: vec![0.8, 0.2, 0.0, 0.0] },
            ChunkRow { chunk_id: "c3".into(), file_id: "f2".into(), chunk_index: 0, start_line: 1, end_line: 3, embedding: vec![0.1, 0.9, 0.0, 0.0] },
            ChunkRow { chunk_id: "c4".into(), file_id: "f2".into(), chunk_index: 1, start_line: 4, end_line: 7, embedding: vec![0.0, 0.8, 0.1, 0.0] },
            ChunkRow { chunk_id: "c5".into(), file_id: "f3".into(), chunk_index: 0, start_line: 1, end_line: 2, embedding: vec![0.0, 0.0, 0.1, 0.9] },
            ChunkRow { chunk_id: "c6".into(), file_id: "f3".into(), chunk_index: 1, start_line: 3, end_line: 5, embedding: vec![0.0, 0.0, 0.2, 0.8] },
        ]
    }

    struct TestIndex {
        _tmp: tempfile::TempDir,
        files_path: std::path::PathBuf,
        chunks_path: std::path::PathBuf,
    }

    fn setup_test_index() -> TestIndex {
        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");

        let (schema_fields, files) = test_files();
        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();

        let chunks_batch = build_chunks_batch(&test_chunks(), 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        TestIndex {
            _tmp: tmp,
            files_path,
            chunks_path,
        }
    }

    #[tokio::test]
    async fn register_and_count() {
        let idx = setup_test_index();
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        let batches = sc
            .query("SELECT COUNT(*) AS cnt FROM chunks")
            .await
            .unwrap();
        let cnt = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0);
        assert_eq!(cnt, 6);
    }

    #[tokio::test]
    async fn chunk_level_search() {
        let idx = setup_test_index();
        let query_vec = vec![1.0, 0.0, 0.0, 0.0]; // rust-like
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        let sql = "
            SELECT f.filepath, c.chunk_id, c.start_line, c.end_line,
                   cosine_similarity(c.embedding) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            ORDER BY score DESC
        ";

        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];

        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        let scores = batch
            .column(4)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        // Top result should be a rust chunk
        assert_eq!(filenames.value(0), "blog/rust.md");
        assert!(scores.value(0) > 0.9);
        // Last results should be cooking (orthogonal to query)
        assert!(scores.value(scores.len() - 1) < 0.1);
    }

    #[tokio::test]
    async fn note_level_ranking() {
        let idx = setup_test_index();
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        let sql = "
            SELECT f.filepath,
                   MAX(cosine_similarity(c.embedding)) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            GROUP BY f.file_id, f.filepath
            ORDER BY score DESC
        ";

        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];

        assert_eq!(batch.num_rows(), 3);
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        let scores = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        assert_eq!(filenames.value(0), "blog/rust.md");
        assert!(scores.value(0) > scores.value(1));
        assert!(scores.value(1) > scores.value(2));
    }

    #[tokio::test]
    async fn frontmatter_filter() {
        let idx = setup_test_index();
        let query_vec = vec![0.0, 0.0, 0.0, 1.0]; // cooking-like
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // Use bare field name via files_v view
        let sql = "
            SELECT f.filepath,
                   MAX(cosine_similarity(c.embedding)) AS score
            FROM chunks c JOIN files_v f ON c.file_id = f.file_id
            WHERE draft = false
            GROUP BY f.file_id, f.filepath
            ORDER BY score DESC
        ";

        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];

        // cooking.md is draft=true, should be filtered out
        assert_eq!(batch.num_rows(), 2);
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        let names: Vec<&str> = (0..filenames.len()).map(|i| filenames.value(i)).collect();
        assert!(!names.contains(&"recipes/cooking.md"));
    }

    #[tokio::test]
    async fn limit() {
        let idx = setup_test_index();
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        let sql = "
            SELECT f.filepath, cosine_similarity(c.embedding) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            ORDER BY score DESC
            LIMIT 2
        ";

        let batches = sc.query(sql).await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);
    }

    #[tokio::test]
    async fn zero_query_vector_returns_zero_scores() {
        let idx = setup_test_index();
        let query_vec = vec![0.0, 0.0, 0.0, 0.0]; // zero vector
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        let sql = "
            SELECT cosine_similarity(c.embedding) AS score
            FROM chunks c
        ";

        let batches = sc.query(sql).await.unwrap();
        let scores = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();

        for i in 0..scores.len() {
            let score = scores.value(i);
            assert_eq!(score, 0.0, "zero query vector should produce 0.0, not NaN");
            assert!(!score.is_nan());
        }
    }

    #[tokio::test]
    async fn limit_exceeds_chunk_count() {
        let idx = setup_test_index();
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // LIMIT 1000 but only 6 chunks exist
        let sql = "
            SELECT f.filepath, cosine_similarity(c.embedding) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            ORDER BY score DESC
            LIMIT 1000
        ";

        let batches = sc.query(sql).await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 6); // returns all, doesn't crash
    }

    #[tokio::test]
    async fn field_name_with_single_quote() {
        let schema_fields = vec![
            ("author's_note".into(), FieldType::String),
            ("draft".into(), FieldType::Boolean),
        ];
        let files = vec![FileRow {
            file_id: "f1".into(),
            filename: "post.md".into(),
            frontmatter: Some(serde_json::json!({"author's_note": "hello world", "draft": false})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let chunks = vec![ChunkRow {
            chunk_id: "c1".into(),
            file_id: "f1".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 3,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }];

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");
        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &files_path,
            &chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // The view should properly escape the single quote in the field name
        let sql = "
            SELECT f.\"author's_note\"
            FROM files_v f
        ";
        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        // Struct accessor returns Utf8, not StringView
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(values.value(0), "hello world");
    }

    #[tokio::test]
    async fn field_name_with_double_quote() {
        let schema_fields = vec![
            ("field\"name".into(), FieldType::String),
            ("draft".into(), FieldType::Boolean),
        ];
        let files = vec![FileRow {
            file_id: "f1".into(),
            filename: "post.md".into(),
            frontmatter: Some(serde_json::json!({"field\"name": "test value", "draft": false})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let chunks = vec![ChunkRow {
            chunk_id: "c1".into(),
            file_id: "f1".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 3,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }];

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");
        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &files_path,
            &chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // The view should properly escape the double quote in the alias
        // To reference it in SQL, we also double the quote
        let sql = "
            SELECT f.\"field\"\"name\"
            FROM files_v f
        ";
        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        // Struct accessor returns Utf8, not StringView
        let values = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(values.value(0), "test value");
    }

    #[tokio::test]
    async fn array_field_filter() {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
        ];
        let files = vec![
            FileRow {
                file_id: "f1".into(),
                filename: "alpha/experiment-1.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": "Baseline calibration", "tags": ["calibration", "SPR-A1"]}),
                ),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "f2".into(),
                filename: "alpha/experiment-3.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": "Environmental sensitivity", "tags": ["calibration", "environment"]}),
                ),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "f3".into(),
                filename: "beta/initial-findings.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": "Kalman filter benchmarks", "tags": ["benchmarks"]}),
                ),
                content_hash: "h3".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];
        let chunks = vec![
            ChunkRow {
                chunk_id: "c1".into(),
                file_id: "f1".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![1.0, 0.0, 0.0, 0.0],
            },
            ChunkRow {
                chunk_id: "c2".into(),
                file_id: "f2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.0, 1.0, 0.0, 0.0],
            },
            ChunkRow {
                chunk_id: "c3".into(),
                file_id: "f3".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.0, 0.0, 1.0, 0.0],
            },
        ];

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");
        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let sc = SearchContext::new(
            &files_path,
            &chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // array_has through the files_v view — should match f1 and f2
        let sql = "
            SELECT f.filepath
            FROM chunks c JOIN files_v f ON c.file_id = f.file_id
            WHERE array_has(tags, 'calibration')
            GROUP BY f.file_id, f.filepath
            ORDER BY f.filepath
        ";
        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "alpha/experiment-1.md");
        assert_eq!(filenames.value(1), "alpha/experiment-3.md");

        // Multiple array_has — should match only f1
        let sql = "
            SELECT f.filepath
            FROM chunks c JOIN files_v f ON c.file_id = f.file_id
            WHERE array_has(tags, 'calibration') AND array_has(tags, 'SPR-A1')
            GROUP BY f.file_id, f.filepath
        ";
        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1);
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "alpha/experiment-1.md");
    }

    #[tokio::test]
    async fn frontmatter_filter_bracket_syntax() {
        let idx = setup_test_index();
        let query_vec = vec![0.0, 0.0, 0.0, 1.0];
        let sc = SearchContext::new(
            &idx.files_path,
            &idx.chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await
        .unwrap();

        // Bracket syntax works via files_v (SELECT * includes data Struct)
        let sql = "
            SELECT f.filepath,
                   MAX(cosine_similarity(c.embedding)) AS score
            FROM chunks c JOIN files_v f ON c.file_id = f.file_id
            WHERE f.data['draft'] = false
            GROUP BY f.file_id, f.filepath
            ORDER BY score DESC
        ";

        let batches = sc.query(sql).await.unwrap();
        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 2);
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .unwrap();
        let names: Vec<&str> = (0..filenames.len()).map(|i| filenames.value(i)).collect();
        assert!(!names.contains(&"recipes/cooking.md"));
    }

    #[tokio::test]
    async fn collision_detected_when_frontmatter_field_matches_internal_column() {
        use crate::index::storage::{build_chunks_batch, build_files_batch, write_parquet};

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");

        // Create a file with a frontmatter field called "filepath" — same as internal column
        let schema_fields = vec![("filepath".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "f1".into(),
            filename: "test.md".into(),
            frontmatter: Some(serde_json::json!({"filepath": "custom/path"})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let chunks = vec![ChunkRow {
            chunk_id: "c1".into(),
            file_id: "f1".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }];

        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let result = SearchContext::new(
            &files_path,
            &chunks_path,
            query_vec,
            "",
            &std::collections::HashMap::new(),
        )
        .await;
        assert!(result.is_err(), "expected collision error");
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("collides with internal column"),
            "unexpected error: {}",
            err
        );
    }

    #[tokio::test]
    async fn collision_resolved_with_prefix() {
        use crate::index::storage::{build_chunks_batch, build_files_batch, write_parquet};

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");

        let schema_fields = vec![("filepath".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "f1".into(),
            filename: "test.md".into(),
            frontmatter: Some(serde_json::json!({"filepath": "custom/path"})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let chunks = vec![ChunkRow {
            chunk_id: "c1".into(),
            file_id: "f1".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }];

        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        // Prefix "_" resolves collision: internal filepath → _filepath
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let result = SearchContext::new(
            &files_path,
            &chunks_path,
            query_vec,
            "_",
            &std::collections::HashMap::new(),
        )
        .await;
        assert!(
            result.is_ok(),
            "prefix should resolve collision: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn collision_resolved_with_alias() {
        use crate::index::storage::{build_chunks_batch, build_files_batch, write_parquet};

        let tmp = tempfile::tempdir().unwrap();
        let files_path = tmp.path().join("files.parquet");
        let chunks_path = tmp.path().join("chunks.parquet");

        let schema_fields = vec![("filepath".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "f1".into(),
            filename: "test.md".into(),
            frontmatter: Some(serde_json::json!({"filepath": "custom/path"})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let chunks = vec![ChunkRow {
            chunk_id: "c1".into(),
            file_id: "f1".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![1.0, 0.0, 0.0, 0.0],
        }];

        let files_batch = build_files_batch(&schema_fields, &files);
        write_parquet(&files_path, &files_batch).unwrap();
        let chunks_batch = build_chunks_batch(&chunks, 4);
        write_parquet(&chunks_path, &chunks_batch).unwrap();

        // Alias resolves collision: internal filepath → "path"
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let mut aliases = std::collections::HashMap::new();
        aliases.insert("filepath".to_string(), "path".to_string());
        let result = SearchContext::new(&files_path, &chunks_path, query_vec, "", &aliases).await;
        assert!(
            result.is_ok(),
            "alias should resolve collision: {:?}",
            result.err()
        );
    }
}
