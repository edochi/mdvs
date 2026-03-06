#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! datafusion = "52"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! ```

use datafusion::arrow::array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, Float64Array, Int32Array,
    Int64Array, StringArray, StringViewArray, StructArray,
};
use datafusion::arrow::datatypes::{DataType, Field, Fields, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::execution::context::SessionContext;
use datafusion::logical_expr::{
    ColumnarValue, ScalarFunctionArgs, ScalarUDF, ScalarUDFImpl, Signature, Volatility,
};
use datafusion::parquet::arrow::ArrowWriter;
use std::any::Any;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use tempfile::tempdir;

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
// Test data builders
// ============================================================================

fn write_files_parquet(path: &std::path::Path) {
    let data_fields = Fields::from(vec![
        Field::new("title", DataType::Utf8, true),
        Field::new("draft", DataType::Boolean, true),
    ]);
    let data_struct_type = DataType::Struct(data_fields.clone());

    let titles = StringArray::from(vec![
        Some("Rust Guide"),
        Some("Python Intro"),
        Some("Cooking Tips"),
    ]);
    let drafts = BooleanArray::from(vec![Some(false), Some(false), Some(true)]);
    let data_arr = StructArray::new(
        data_fields,
        vec![Arc::new(titles) as ArrayRef, Arc::new(drafts) as ArrayRef],
        None,
    );

    let schema = Schema::new(vec![
        Field::new("file_id", DataType::Utf8, false),
        Field::new("filename", DataType::Utf8, false),
        Field::new("data", data_struct_type, true),
        Field::new("content_hash", DataType::Utf8, false),
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(StringArray::from(vec!["f1", "f2", "f3"])),
            Arc::new(StringArray::from(vec![
                "blog/rust.md",
                "blog/python.md",
                "recipes/cooking.md",
            ])),
            Arc::new(data_arr),
            Arc::new(StringArray::from(vec!["h1", "h2", "h3"])),
        ],
    )
    .unwrap();

    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn write_chunks_parquet(path: &std::path::Path, dimension: i32) {
    // 6 chunks across 3 files
    // f1 (rust): 2 chunks with rust-like embeddings
    // f2 (python): 2 chunks with python-like embeddings
    // f3 (cooking): 2 chunks with cooking-like embeddings
    let chunk_ids = StringArray::from(vec!["c1", "c2", "c3", "c4", "c5", "c6"]);
    let file_ids = StringArray::from(vec!["f1", "f1", "f2", "f2", "f3", "f3"]);
    let chunk_indices = Int32Array::from(vec![0, 1, 0, 1, 0, 1]);
    let start_lines = Int32Array::from(vec![1, 5, 1, 4, 1, 3]);
    let end_lines = Int32Array::from(vec![4, 8, 3, 7, 2, 5]);

    // Embeddings: make rust chunks similar to each other, different from cooking
    #[rustfmt::skip]
    let flat_embeddings: Vec<f32> = vec![
        0.9, 0.1, 0.0, 0.0,  // c1: rust-like
        0.8, 0.2, 0.0, 0.0,  // c2: rust-like
        0.1, 0.9, 0.0, 0.0,  // c3: python-like
        0.0, 0.8, 0.1, 0.0,  // c4: python-like
        0.0, 0.0, 0.1, 0.9,  // c5: cooking-like
        0.0, 0.0, 0.2, 0.8,  // c6: cooking-like
    ];

    let values = Float32Array::from(flat_embeddings);
    let embedding_arr = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        dimension,
        Arc::new(values),
        None,
    );

    let schema = Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("file_id", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("start_line", DataType::Int32, false),
        Field::new("end_line", DataType::Int32, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, false)),
                dimension,
            ),
            false,
        ),
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(chunk_ids),
            Arc::new(file_ids),
            Arc::new(chunk_indices),
            Arc::new(start_lines),
            Arc::new(end_lines),
            Arc::new(embedding_arr),
        ],
    )
    .unwrap();

    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

// ============================================================================
// Tests
// ============================================================================

#[tokio::main]
async fn main() {
    println!("=== Search tests ===\n");

    let tmp = tempdir().unwrap();
    let dimension = 4i32;

    let files_path = tmp.path().join("files.parquet");
    let chunks_path = tmp.path().join("chunks.parquet");
    write_files_parquet(&files_path);
    write_chunks_parquet(&chunks_path, dimension);

    // --- Test 1: Register tables and run basic query ---
    {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();

        let df = ctx.sql("SELECT COUNT(*) AS cnt FROM chunks").await.unwrap();
        let batches = df.collect().await.unwrap();
        let cnt = batches[0]
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap()
            .value(0);
        assert_eq!(cnt, 6);
        println!("  1. Registered tables, 6 chunks  ✓");
    }

    // --- Test 2: Cosine similarity UDF — chunk-level search ---
    {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();

        // Query vector similar to "rust" embeddings
        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_vec));
        ctx.register_udf(udf);

        let sql = "
            SELECT f.filename, c.chunk_id, c.start_line, c.end_line,
                   cosine_similarity(c.embedding) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            ORDER BY score DESC
        ";

        let df = ctx.sql(sql).await.unwrap();
        let batches = df.collect().await.unwrap();
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

        println!(
            "  2. Chunk-level search: top={} score={:.4}  ✓",
            filenames.value(0),
            scores.value(0)
        );
    }

    // --- Test 3: Note-level ranking (MAX similarity per file) ---
    {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_vec));
        ctx.register_udf(udf);

        let sql = "
            SELECT f.filename,
                   MAX(cosine_similarity(c.embedding)) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            GROUP BY f.file_id, f.filename
            ORDER BY score DESC
        ";

        let df = ctx.sql(sql).await.unwrap();
        let batches = df.collect().await.unwrap();
        let batch = &batches[0];

        assert_eq!(batch.num_rows(), 3); // 3 files
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

        println!(
            "  3. Note-level ranking: {} > {} > {}  ✓",
            filenames.value(0),
            filenames.value(1),
            filenames.value(2)
        );
    }

    // --- Test 4: Filter by frontmatter (WHERE) ---
    {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();

        let query_vec = vec![0.0, 0.0, 0.0, 1.0]; // cooking-like query
        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_vec));
        ctx.register_udf(udf);

        // Filter out drafts
        let sql = "
            SELECT f.filename,
                   MAX(cosine_similarity(c.embedding)) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            WHERE f.data['draft'] = false
            GROUP BY f.file_id, f.filename
            ORDER BY score DESC
        ";

        let df = ctx.sql(sql).await.unwrap();
        let batches = df.collect().await.unwrap();
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

        println!(
            "  4. Frontmatter filter (draft=false): {} results  ✓",
            batch.num_rows()
        );
    }

    // --- Test 5: LIMIT ---
    {
        let ctx = SessionContext::new();
        ctx.register_parquet("files", files_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();
        ctx.register_parquet("chunks", chunks_path.to_str().unwrap(), Default::default())
            .await
            .unwrap();

        let query_vec = vec![1.0, 0.0, 0.0, 0.0];
        let udf = ScalarUDF::from(CosineSimilarityUDF::new(query_vec));
        ctx.register_udf(udf);

        let sql = "
            SELECT f.filename, cosine_similarity(c.embedding) AS score
            FROM chunks c JOIN files f ON c.file_id = f.file_id
            ORDER BY score DESC
            LIMIT 2
        ";

        let df = ctx.sql(sql).await.unwrap();
        let batches = df.collect().await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        println!("  5. LIMIT 2: {} results  ✓", total_rows);
    }

    println!("\n=== All tests passed ===");
}
