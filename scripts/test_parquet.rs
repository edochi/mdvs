#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! arrow = "54"
//! parquet = "54"
//! serde_json = "1"
//! tempfile = "3"
//! ```

use arrow::array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, Float64Array, Int64Array,
    ListArray, StringArray, StructArray,
};
use arrow::buffer::{NullBuffer, OffsetBuffer};
use arrow::datatypes::{DataType, Field, Fields, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::sync::Arc;

// ============================================================================
// FieldType + Arrow mapping (from test_arrow.rs)
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum FieldType {
    Boolean,
    Integer,
    Float,
    String,
    Array(Box<FieldType>),
    Object(BTreeMap<std::string::String, FieldType>),
}

impl Into<DataType> for &FieldType {
    fn into(self) -> DataType {
        match self {
            FieldType::Boolean => DataType::Boolean,
            FieldType::Integer => DataType::Int64,
            FieldType::Float => DataType::Float64,
            FieldType::String => DataType::Utf8,
            FieldType::Array(inner) => {
                let inner_dt: DataType = inner.as_ref().into();
                DataType::List(Arc::new(Field::new("item", inner_dt, true)))
            }
            FieldType::Object(fields) => {
                let arrow_fields: Vec<Field> = fields
                    .iter()
                    .map(|(name, ft)| Field::new(name, ft.into(), true))
                    .collect();
                DataType::Struct(Fields::from(arrow_fields))
            }
        }
    }
}

// ============================================================================
// build_array (from test_arrow.rs)
// ============================================================================

fn build_array(values: &[Option<&Value>], ft: &FieldType) -> ArrayRef {
    match ft {
        FieldType::Boolean => {
            let arr: BooleanArray = values.iter().map(|v| v.and_then(|v| v.as_bool())).collect();
            Arc::new(arr)
        }
        FieldType::Integer => {
            let arr: Int64Array = values.iter().map(|v| v.and_then(|v| v.as_i64())).collect();
            Arc::new(arr)
        }
        FieldType::Float => {
            let arr: Float64Array = values
                .iter()
                .map(|v| v.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64))))
                .collect();
            Arc::new(arr)
        }
        FieldType::String => {
            let arr: StringArray = values.iter().map(|v| v.and_then(|v| v.as_str())).collect();
            Arc::new(arr)
        }
        FieldType::Array(inner) => {
            let mut offsets: Vec<i32> = vec![0];
            let mut child_values: Vec<Option<&Value>> = Vec::new();
            let mut nulls: Vec<bool> = Vec::new();
            for v in values {
                match v.and_then(|v| v.as_array()) {
                    Some(arr) => {
                        for elem in arr {
                            child_values.push(Some(elem));
                        }
                        offsets.push(child_values.len() as i32);
                        nulls.push(true);
                    }
                    None => {
                        offsets.push(*offsets.last().unwrap());
                        nulls.push(false);
                    }
                }
            }
            let child_array = build_array(&child_values, inner);
            let inner_dt: DataType = inner.as_ref().into();
            Arc::new(ListArray::new(
                Arc::new(Field::new("item", inner_dt, true)),
                OffsetBuffer::new(offsets.into()),
                child_array,
                Some(NullBuffer::from(nulls)),
            ))
        }
        FieldType::Object(fields) => {
            let nulls: Vec<bool> = values
                .iter()
                .map(|v| v.and_then(|v| v.as_object()).is_some())
                .collect();
            let children: Vec<(Arc<Field>, ArrayRef)> = fields
                .iter()
                .map(|(name, sub_ft)| {
                    let sub_values: Vec<Option<&Value>> = values
                        .iter()
                        .map(|v| v.and_then(|v| v.get(name.as_str())))
                        .collect();
                    let sub_dt: DataType = sub_ft.into();
                    (
                        Arc::new(Field::new(name, sub_dt, true)),
                        build_array(&sub_values, sub_ft),
                    )
                })
                .collect();
            let (child_fields, child_arrays): (Vec<_>, Vec<_>) = children.into_iter().unzip();
            Arc::new(StructArray::new(
                child_fields.into(),
                child_arrays,
                Some(NullBuffer::from(nulls)),
            ))
        }
    }
}

// ============================================================================
// files.parquet helpers
// ============================================================================

struct FileRow {
    file_id: std::string::String,
    filename: std::string::String,
    frontmatter: Option<Value>,
    content_hash: std::string::String,
    built_at: std::string::String,
}

fn build_files_batch(
    schema_fields: &[(std::string::String, FieldType)],
    files: &[FileRow],
) -> RecordBatch {
    let file_id_arr: StringArray = files.iter().map(|f| Some(f.file_id.as_str())).collect();
    let filename_arr: StringArray = files.iter().map(|f| Some(f.filename.as_str())).collect();
    let content_hash_arr: StringArray =
        files.iter().map(|f| Some(f.content_hash.as_str())).collect();
    let built_at_arr: StringArray = files.iter().map(|f| Some(f.built_at.as_str())).collect();

    let mut data_child_fields: Vec<Arc<Field>> = Vec::new();
    let mut data_child_arrays: Vec<ArrayRef> = Vec::new();
    for (name, ft) in schema_fields {
        let values: Vec<Option<&Value>> = files
            .iter()
            .map(|f| f.frontmatter.as_ref().and_then(|obj| obj.get(name.as_str())))
            .collect();
        let dt: DataType = ft.into();
        data_child_fields.push(Arc::new(Field::new(name, dt, true)));
        data_child_arrays.push(build_array(&values, ft));
    }

    let data_nulls: Vec<bool> = files.iter().map(|f| f.frontmatter.is_some()).collect();
    let data_struct_type = DataType::Struct(Fields::from(
        data_child_fields
            .iter()
            .map(|f| f.as_ref().clone())
            .collect::<Vec<_>>(),
    ));
    let data_arr = StructArray::new(
        data_child_fields.into(),
        data_child_arrays,
        Some(NullBuffer::from(data_nulls)),
    );

    let schema = Schema::new(vec![
        Field::new("file_id", DataType::Utf8, false),
        Field::new("filename", DataType::Utf8, false),
        Field::new("data", data_struct_type, true),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new("built_at", DataType::Utf8, false),
    ]);

    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(file_id_arr),
            Arc::new(filename_arr),
            Arc::new(data_arr),
            Arc::new(content_hash_arr),
            Arc::new(built_at_arr),
        ],
    )
    .unwrap()
}

// ============================================================================
// chunks.parquet helpers
// ============================================================================

struct ChunkRow {
    chunk_id: std::string::String,
    file_id: std::string::String,
    chunk_index: i32,
    start_line: i32,
    end_line: i32,
    embedding: Vec<f32>,
}

fn build_chunks_batch(chunks: &[ChunkRow], dimension: i32) -> RecordBatch {
    let chunk_id_arr: StringArray = chunks.iter().map(|c| Some(c.chunk_id.as_str())).collect();
    let file_id_arr: StringArray = chunks.iter().map(|c| Some(c.file_id.as_str())).collect();
    let chunk_index_arr: arrow::array::Int32Array =
        chunks.iter().map(|c| Some(c.chunk_index)).collect();
    let start_line_arr: arrow::array::Int32Array =
        chunks.iter().map(|c| Some(c.start_line)).collect();
    let end_line_arr: arrow::array::Int32Array = chunks.iter().map(|c| Some(c.end_line)).collect();

    // Embedding: FixedSizeList<Float32>(dimension)
    let flat_values: Vec<f32> = chunks.iter().flat_map(|c| c.embedding.iter().copied()).collect();
    let values_arr = Float32Array::from(flat_values);
    let embedding_arr = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        dimension,
        Arc::new(values_arr),
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

    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(chunk_id_arr),
            Arc::new(file_id_arr),
            Arc::new(chunk_index_arr),
            Arc::new(start_line_arr),
            Arc::new(end_line_arr),
            Arc::new(embedding_arr),
        ],
    )
    .unwrap()
}

// ============================================================================
// Tests
// ============================================================================

fn main() {
    println!("=== Parquet I/O tests ===\n");

    let tmp = tempfile::tempdir().unwrap();

    // --- Test 1: Write and read files.parquet ---
    {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            ("draft".into(), FieldType::Boolean),
        ];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "blog/post1.md".into(),
                frontmatter: Some(serde_json::json!({
                    "title": "Hello", "tags": ["rust", "arrow"], "draft": false
                })),
                content_hash: "hash1".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "blog/post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "World"})),
                content_hash: "hash2".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "notes/bare.md".into(),
                frontmatter: None,
                content_hash: "hash3".into(),
                built_at: "2025-01-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        let path = tmp.path().join("files.parquet");

        // Write
        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        // Read back
        let file = File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let batches: Vec<RecordBatch> = reader.map(|r| r.unwrap()).collect();
        assert_eq!(batches.len(), 1);
        let read_batch = &batches[0];
        assert_eq!(read_batch.num_rows(), 3);

        // Verify data column
        let data = read_batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let titles = data
            .column_by_name("title")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(titles.value(0), "Hello");
        assert_eq!(titles.value(1), "World");
        assert!(titles.is_null(2));

        let tags = data
            .column_by_name("tags")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let row0_tags = tags.value(0);
        let row0_strs = row0_tags.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(row0_strs.value(0), "rust");
        assert_eq!(row0_strs.value(1), "arrow");
        assert!(tags.is_null(1));

        println!("  1. files.parquet write + read roundtrip  ✓");
    }

    // --- Test 2: Write and read chunks.parquet ---
    {
        let dimension = 4; // small for testing
        let chunks = vec![
            ChunkRow {
                chunk_id: "c1".into(),
                file_id: "id-1".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 5,
                embedding: vec![0.1, 0.2, 0.3, 0.4],
            },
            ChunkRow {
                chunk_id: "c2".into(),
                file_id: "id-1".into(),
                chunk_index: 1,
                start_line: 7,
                end_line: 12,
                embedding: vec![0.5, 0.6, 0.7, 0.8],
            },
            ChunkRow {
                chunk_id: "c3".into(),
                file_id: "id-2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.9, 1.0, 1.1, 1.2],
            },
        ];

        let batch = build_chunks_batch(&chunks, dimension);
        let path = tmp.path().join("chunks.parquet");

        // Write
        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();

        // Read back
        let file = File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let batches: Vec<RecordBatch> = reader.map(|r| r.unwrap()).collect();
        assert_eq!(batches.len(), 1);
        let read_batch = &batches[0];
        assert_eq!(read_batch.num_rows(), 3);

        // Verify chunk_id
        let chunk_ids = read_batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(chunk_ids.value(0), "c1");
        assert_eq!(chunk_ids.value(2), "c3");

        // Verify embedding
        let embeddings = read_batch
            .column(5)
            .as_any()
            .downcast_ref::<FixedSizeListArray>()
            .unwrap();
        let row0_emb = embeddings.value(0);
        let row0_floats = row0_emb.as_any().downcast_ref::<Float32Array>().unwrap();
        assert_eq!(row0_floats.len(), 4);
        assert!((row0_floats.value(0) - 0.1).abs() < f32::EPSILON);
        assert!((row0_floats.value(3) - 0.4).abs() < f32::EPSILON);

        println!("  2. chunks.parquet write + read roundtrip  ✓");
    }

    // --- Test 3: Streaming write (multiple batches → multiple row groups) ---
    {
        let schema_fields: Vec<(std::string::String, FieldType)> =
            vec![("title".into(), FieldType::String)];

        let path = tmp.path().join("files_streamed.parquet");

        // Build two batches
        let batch1 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "a".into(),
                filename: "a.md".into(),
                frontmatter: Some(serde_json::json!({"title": "First"})),
                content_hash: "h1".into(),
                built_at: "2025-01-01".into(),
            }],
        );
        let batch2 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "b".into(),
                filename: "b.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Second"})),
                content_hash: "h2".into(),
                built_at: "2025-01-01".into(),
            }],
        );

        // Write both batches (each becomes a row group)
        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, batch1.schema(), None).unwrap();
        writer.write(&batch1).unwrap();
        writer.write(&batch2).unwrap();
        writer.close().unwrap();

        // Read back — should get 2 rows total
        let file = File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .build()
            .unwrap();

        let all_batches: Vec<RecordBatch> = reader.map(|r| r.unwrap()).collect();
        let total_rows: usize = all_batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        // Verify data from both batches
        let first_batch = &all_batches[0];
        let filenames = first_batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "a.md");

        let last_batch = all_batches.last().unwrap();
        let filenames = last_batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(filenames.len() - 1), "b.md");

        println!("  3. Streaming write (multiple row groups)  ✓");
    }

    // --- Test 4: Streaming read (iterate without collecting) ---
    {
        let path = tmp.path().join("files_streamed.parquet");
        let file = File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .with_batch_size(1) // read one row at a time
            .build()
            .unwrap();

        let mut row_count = 0;
        for batch in reader {
            let batch = batch.unwrap();
            row_count += batch.num_rows();
        }
        assert_eq!(row_count, 2);
        println!("  4. Streaming read (batch_size=1)  ✓");
    }

    // --- Test 5: Column projection (read only specific columns) ---
    {
        let path = tmp.path().join("files.parquet");
        let file = File::open(&path).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();

        // Read only filename and content_hash (columns 1 and 3)
        let mask = parquet::arrow::ProjectionMask::roots(
            builder.parquet_schema(),
            [1, 3],
        );
        let reader = builder.with_projection(mask).build().unwrap();

        let batches: Vec<RecordBatch> = reader.map(|r| r.unwrap()).collect();
        let batch = &batches[0];
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.schema().field(0).name(), "filename");
        assert_eq!(batch.schema().field(1).name(), "content_hash");

        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "blog/post1.md");

        println!("  5. Column projection (read 2 of 5 columns)  ✓");
    }

    // --- Test 6: File size is reasonable ---
    {
        let path = tmp.path().join("files.parquet");
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 0);
        assert!(size < 10_000); // 3 rows should be tiny
        println!("  6. File size: {} bytes  ✓", size);
    }

    println!("\n=== All tests passed ===");
}
