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

        Ok(ColumnarValue::from(Arc::new(results) as ArrayRef))
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
    /// Register Parquet files as tables and bind the query embedding into the UDF.
    #[instrument(name = "register_tables", skip_all, level = "debug")]
    pub async fn new(
        files_path: &Path,
        chunks_path: &Path,
        query_embedding: Vec<f32>,
    ) -> anyhow::Result<Self> {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await?;
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await?;

        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_embedding));
        ctx.register_udf(udf);

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
    use crate::index::storage::{build_chunks_batch, build_files_batch, write_parquet, ChunkRow, FileRow};
    use datafusion::arrow::array::{Array, Float64Array, Int64Array, StringViewArray};

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
        let files_batch = build_files_batch(&schema_fields, &files, "_");
        write_parquet(&files_path, &files_batch).unwrap();

        let chunks_batch = build_chunks_batch(&test_chunks(), 4, "_");
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
        let sc = SearchContext::new(&idx.files_path, &idx.chunks_path, query_vec)
            .await
            .unwrap();

        let batches = sc.query("SELECT COUNT(*) AS cnt FROM chunks").await.unwrap();
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
        let sc = SearchContext::new(&idx.files_path, &idx.chunks_path, query_vec)
            .await
            .unwrap();

        let sql = "
            SELECT f._filename, c._chunk_id, c._start_line, c._end_line,
                   cosine_similarity(c._embedding) AS score
            FROM chunks c JOIN files f ON c._file_id = f._file_id
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
        let sc = SearchContext::new(&idx.files_path, &idx.chunks_path, query_vec)
            .await
            .unwrap();

        let sql = "
            SELECT f._filename,
                   MAX(cosine_similarity(c._embedding)) AS score
            FROM chunks c JOIN files f ON c._file_id = f._file_id
            GROUP BY f._file_id, f._filename
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
        let sc = SearchContext::new(&idx.files_path, &idx.chunks_path, query_vec)
            .await
            .unwrap();

        let sql = "
            SELECT f._filename,
                   MAX(cosine_similarity(c._embedding)) AS score
            FROM chunks c JOIN files f ON c._file_id = f._file_id
            WHERE f._data['draft'] = false
            GROUP BY f._file_id, f._filename
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
        let sc = SearchContext::new(&idx.files_path, &idx.chunks_path, query_vec)
            .await
            .unwrap();

        let sql = "
            SELECT f._filename, cosine_similarity(c._embedding) AS score
            FROM chunks c JOIN files f ON c._file_id = f._file_id
            ORDER BY score DESC
            LIMIT 2
        ";

        let batches = sc.query(sql).await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);
    }
}
