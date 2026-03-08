use crate::discover::field_type::FieldType;
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use xxhash_rust::xxh3::xxh3_64;

use datafusion::arrow::array::{
    ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, Float64Array, Int32Array, Int64Array,
    ListArray, StringArray, StructArray, TimestampMicrosecondArray,
};
use datafusion::arrow::buffer::{NullBuffer, OffsetBuffer};
use datafusion::arrow::datatypes::{DataType, Field, Fields, Schema, TimeUnit};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use datafusion::parquet::arrow::ArrowWriter;
use datafusion::parquet::arrow::ProjectionMask;
use datafusion::parquet::basic::Compression;
use datafusion::parquet::file::properties::WriterProperties;
use serde_json::Value;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;

/// Prepend `prefix` to a base column name (e.g. `col("_", "file_id")` → `"_file_id"`).
pub fn col(prefix: &str, name: &str) -> String {
    format!("{prefix}{name}")
}

/// Internal column base names in `files.parquet` that must not collide with frontmatter fields.
const RESERVED_BASE_NAMES: &[&str] = &["file_id", "filename", "data", "content_hash", "built_at"];

/// Verify that no frontmatter field name collides with a reserved internal column name.
///
/// Returns an error if any field name matches `{prefix}{base}` for any reserved base name.
pub fn check_reserved_names(field_names: &[String], prefix: &str) -> anyhow::Result<()> {
    for name in field_names {
        for base in RESERVED_BASE_NAMES {
            if *name == col(prefix, base) {
                anyhow::bail!(
                    "field '{}' conflicts with reserved internal column name \
                     (set internal_prefix in [storage] in mdvs.toml to avoid this)",
                    name
                );
            }
        }
    }
    Ok(())
}

/// Compute a deterministic hex-encoded hash of the given content using xxh3.
pub fn content_hash(content: &str) -> String {
    format!("{:016x}", xxh3_64(content.as_bytes()))
}

/// A single file's metadata, ready to be written into `files.parquet`.
pub struct FileRow {
    /// Unique identifier for this file (UUID).
    pub file_id: String,
    /// Path relative to the project root.
    pub filename: String,
    /// Parsed YAML frontmatter as JSON, or `None` for bare files.
    pub frontmatter: Option<Value>,
    /// SipHash of the markdown body (excluding frontmatter).
    pub content_hash: String,
    /// Timestamp when this file was indexed, as microseconds since epoch.
    pub built_at: i64,
}

/// A single chunk with its embedding, ready to be written into `chunks.parquet`.
pub struct ChunkRow {
    /// Unique identifier for this chunk (UUID).
    pub chunk_id: String,
    /// Foreign key referencing `FileRow::file_id`.
    pub file_id: String,
    /// Zero-based index of this chunk within its parent file.
    pub chunk_index: i32,
    /// First line of this chunk in the source file (1-based).
    pub start_line: i32,
    /// Last line of this chunk in the source file (1-based, inclusive).
    pub end_line: i32,
    /// Dense embedding vector for this chunk.
    pub embedding: Vec<f32>,
}

/// Configuration snapshot stored in parquet key-value metadata.
///
/// Captures the exact settings used for a build so that subsequent builds
/// and searches can detect config changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildMetadata {
    /// Embedding model configuration (provider, name, revision).
    pub embedding_model: EmbeddingModelConfig,
    /// Chunking configuration (max chunk size).
    pub chunking: ChunkingConfig,
    /// Glob pattern used to select files for the build.
    pub glob: String,
    /// ISO 8601 timestamp of when the build was produced.
    pub built_at: String,
    /// Prefix applied to internal parquet column names.
    pub internal_prefix: String,
}

impl BuildMetadata {
    /// Serialize to a `HashMap` suitable for parquet key-value metadata.
    pub fn to_hash_map(&self) -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert(
            "mdvs.provider".into(),
            self.embedding_model.provider.clone(),
        );
        m.insert("mdvs.model".into(), self.embedding_model.name.clone());
        if let Some(ref r) = self.embedding_model.revision {
            m.insert("mdvs.revision".into(), r.clone());
        }
        m.insert(
            "mdvs.chunk_size".into(),
            self.chunking.max_chunk_size.to_string(),
        );
        m.insert("mdvs.glob".into(), self.glob.clone());
        m.insert("mdvs.built_at".into(), self.built_at.clone());
        m.insert("mdvs.internal_prefix".into(), self.internal_prefix.clone());
        m
    }

    /// Deserialize from parquet key-value metadata. Returns `None` if required keys are missing.
    pub fn from_hash_map(meta: &HashMap<String, String>) -> Option<Self> {
        Some(Self {
            embedding_model: EmbeddingModelConfig {
                provider: meta
                    .get("mdvs.provider")
                    .cloned()
                    .unwrap_or_else(|| "model2vec".to_string()),
                name: meta.get("mdvs.model")?.clone(),
                revision: meta.get("mdvs.revision").cloned(),
            },
            chunking: ChunkingConfig {
                max_chunk_size: meta.get("mdvs.chunk_size")?.parse().ok()?,
            },
            glob: meta.get("mdvs.glob")?.clone(),
            built_at: meta.get("mdvs.built_at")?.clone(),
            // Missing key means pre-prefix parquet (old format) — use empty string
            // so comparison with default "_" detects mismatch and requires --force
            internal_prefix: meta
                .get("mdvs.internal_prefix")
                .cloned()
                .unwrap_or_default(),
        })
    }
}

// ============================================================================
// Arrow array builder (recursive, from JSON + FieldType)
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
            let arr: StringArray = values
                .iter()
                .map(|v| {
                    v.and_then(|v| match v {
                        Value::Null => None,
                        Value::String(s) => Some(s.clone()),
                        other => Some(other.to_string()),
                    })
                })
                .collect();
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
// Batch builders
// ============================================================================

/// Build an Arrow `RecordBatch` for `files.parquet` from file rows and the inferred schema.
///
/// Frontmatter values are packed into a single `data` Struct column whose
/// child fields match `schema_fields`.
pub fn build_files_batch(
    schema_fields: &[(String, FieldType)],
    files: &[FileRow],
    prefix: &str,
) -> RecordBatch {
    let file_id_arr: StringArray = files.iter().map(|f| Some(f.file_id.as_str())).collect();
    let filename_arr: StringArray = files.iter().map(|f| Some(f.filename.as_str())).collect();
    let content_hash_arr: StringArray = files
        .iter()
        .map(|f| Some(f.content_hash.as_str()))
        .collect();
    let built_at_arr: TimestampMicrosecondArray = files.iter().map(|f| Some(f.built_at)).collect();

    let mut data_child_fields: Vec<Arc<Field>> = Vec::new();
    let mut data_child_arrays: Vec<ArrayRef> = Vec::new();
    for (name, ft) in schema_fields {
        let values: Vec<Option<&Value>> = files
            .iter()
            .map(|f| {
                f.frontmatter
                    .as_ref()
                    .and_then(|obj| obj.get(name.as_str()))
            })
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
        Field::new(col(prefix, "file_id"), DataType::Utf8, false),
        Field::new(col(prefix, "filename"), DataType::Utf8, false),
        Field::new(col(prefix, "data"), data_struct_type, true),
        Field::new(col(prefix, "content_hash"), DataType::Utf8, false),
        Field::new(
            col(prefix, "built_at"),
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
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

/// Build an Arrow `RecordBatch` for `chunks.parquet` from chunk rows.
///
/// Embeddings are stored as a `FixedSizeList<Float32>` with the given `dimension`.
pub fn build_chunks_batch(chunks: &[ChunkRow], dimension: i32, prefix: &str) -> RecordBatch {
    let chunk_id_arr: StringArray = chunks.iter().map(|c| Some(c.chunk_id.as_str())).collect();
    let file_id_arr: StringArray = chunks.iter().map(|c| Some(c.file_id.as_str())).collect();
    let chunk_index_arr: Int32Array = chunks.iter().map(|c| Some(c.chunk_index)).collect();
    let start_line_arr: Int32Array = chunks.iter().map(|c| Some(c.start_line)).collect();
    let end_line_arr: Int32Array = chunks.iter().map(|c| Some(c.end_line)).collect();

    let flat_values: Vec<f32> = chunks
        .iter()
        .flat_map(|c| c.embedding.iter().copied())
        .collect();
    let values_arr = Float32Array::from(flat_values);
    let embedding_arr = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        dimension,
        Arc::new(values_arr),
        None,
    );

    let schema = Schema::new(vec![
        Field::new(col(prefix, "chunk_id"), DataType::Utf8, false),
        Field::new(col(prefix, "file_id"), DataType::Utf8, false),
        Field::new(col(prefix, "chunk_index"), DataType::Int32, false),
        Field::new(col(prefix, "start_line"), DataType::Int32, false),
        Field::new(col(prefix, "end_line"), DataType::Int32, false),
        Field::new(
            col(prefix, "embedding"),
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
// Parquet I/O
// ============================================================================

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build()
}

/// Write a single `RecordBatch` to a Snappy-compressed Parquet file.
pub fn write_parquet(path: &Path, batch: &RecordBatch) -> anyhow::Result<()> {
    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(batch)?;
    writer.close()?;
    Ok(())
}

/// Read all `RecordBatch`es from a Parquet file.
pub fn read_parquet(path: &Path) -> anyhow::Result<Vec<RecordBatch>> {
    let file = File::open(path)?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;
    let batches: Vec<RecordBatch> = reader.collect::<Result<Vec<_>, _>>()?;
    Ok(batches)
}

/// Write a `RecordBatch` to Parquet, attaching key-value metadata to the Arrow schema.
pub fn write_parquet_with_metadata(
    path: &Path,
    batch: &RecordBatch,
    metadata: HashMap<String, String>,
) -> anyhow::Result<()> {
    let schema = (*batch.schema()).clone().with_metadata(metadata);
    let batch = batch.clone().with_schema(Arc::new(schema))?;
    let file = File::create(path)?;
    let mut writer = ArrowWriter::try_new(file, batch.schema(), Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// Read `BuildMetadata` from a Parquet file's schema-level key-value metadata.
/// Returns `Ok(None)` if the file exists but contains no mdvs metadata keys.
pub fn read_build_metadata(path: &Path) -> anyhow::Result<Option<BuildMetadata>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema();
    Ok(BuildMetadata::from_hash_map(schema.metadata()))
}

// ============================================================================
// Incremental build readers
// ============================================================================

/// Lightweight view of a file row for incremental build diffing.
/// Does NOT include frontmatter — that comes from the fresh scan.
pub struct FileIndexEntry {
    /// Unique identifier for this file.
    pub file_id: String,
    /// Path relative to the project root.
    pub filename: String,
    /// xxh3 hash of the markdown body, used to detect content changes.
    pub content_hash: String,
}

/// Read file_id, filename, content_hash from files.parquet using column projection.
/// Skips the data Struct column (column 2) and built_at (column 4).
pub fn read_file_index(path: &Path) -> anyhow::Result<Vec<FileIndexEntry>> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let mask = ProjectionMask::roots(builder.parquet_schema(), [0, 1, 3]);
    let reader = builder.with_projection(mask).build()?;

    let mut entries = Vec::new();
    for batch in reader {
        let batch = batch?;
        let file_ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("expected StringArray for file_id"))?;
        let filenames = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("expected StringArray for filename"))?;
        let hashes = batch
            .column(2)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("expected StringArray for content_hash"))?;
        for i in 0..batch.num_rows() {
            entries.push(FileIndexEntry {
                file_id: file_ids.value(i).to_string(),
                filename: filenames.value(i).to_string(),
                content_hash: hashes.value(i).to_string(),
            });
        }
    }
    Ok(entries)
}

/// Deserialize all rows from chunks.parquet back into ChunkRow structs.
pub fn read_chunk_rows(path: &Path) -> anyhow::Result<Vec<ChunkRow>> {
    let batches = read_parquet(path)?;
    let mut rows = Vec::new();
    for batch in &batches {
        let chunk_ids = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("expected StringArray for chunk_id"))?;
        let file_ids = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| anyhow::anyhow!("expected StringArray for file_id"))?;
        let chunk_indices = batch
            .column(2)
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| anyhow::anyhow!("expected Int32Array for chunk_index"))?;
        let start_lines = batch
            .column(3)
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| anyhow::anyhow!("expected Int32Array for start_line"))?;
        let end_lines = batch
            .column(4)
            .as_any()
            .downcast_ref::<Int32Array>()
            .ok_or_else(|| anyhow::anyhow!("expected Int32Array for end_line"))?;
        let embeddings = batch
            .column(5)
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
                embedding,
            });
        }
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::Array;

    #[test]
    fn files_parquet_roundtrip() {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
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
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "blog/post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "World"})),
                content_hash: "hash2".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "notes/bare.md".into(),
                frontmatter: None,
                content_hash: "hash3".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");

        write_parquet(&path, &batch).unwrap();
        let batches = read_parquet(&path).unwrap();

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
    }

    #[test]
    fn string_field_serializes_non_strings() {
        let schema_fields = vec![("meta".into(), FieldType::String)];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "a.md".into(),
                frontmatter: Some(serde_json::json!({"meta": "plain text"})),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "b.md".into(),
                frontmatter: Some(serde_json::json!({"meta": ["a", "b"]})),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "c.md".into(),
                frontmatter: Some(serde_json::json!({"meta": {"k": "v"}})),
                content_hash: "h3".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-4".into(),
                filename: "d.md".into(),
                frontmatter: Some(serde_json::json!({"meta": true})),
                content_hash: "h4".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-5".into(),
                filename: "e.md".into(),
                frontmatter: Some(serde_json::json!({"meta": 42})),
                content_hash: "h5".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files, "_");

        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let meta_col = data
            .column_by_name("meta")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();

        // Actual string preserved as-is
        assert_eq!(meta_col.value(0), "plain text");
        // Array serialized to JSON
        assert_eq!(meta_col.value(1), r#"["a","b"]"#);
        // Object serialized to JSON
        assert_eq!(meta_col.value(2), r#"{"k":"v"}"#);
        // Boolean serialized
        assert_eq!(meta_col.value(3), "true");
        // Number serialized
        assert_eq!(meta_col.value(4), "42");

        // No nulls — nothing dropped
        for i in 0..5 {
            assert!(!meta_col.is_null(i), "row {i} should not be null");
        }
    }

    #[test]
    fn null_values_become_arrow_null() {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("count".into(), FieldType::Integer),
            ("score".into(), FieldType::Float),
            ("draft".into(), FieldType::Boolean),
        ];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "a.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": null, "count": null, "score": null, "draft": null}),
                ),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "b.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": "hello", "count": 5, "score": 3.14, "draft": true}),
                ),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files, "_");

        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();

        let title = data
            .column_by_name("title")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let count = data
            .column_by_name("count")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        let score = data
            .column_by_name("score")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        let draft = data
            .column_by_name("draft")
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();

        // Row 0: all null values → Arrow NULL
        assert!(title.is_null(0), "null title should be Arrow NULL");
        assert!(count.is_null(0), "null count should be Arrow NULL");
        assert!(score.is_null(0), "null score should be Arrow NULL");
        assert!(draft.is_null(0), "null draft should be Arrow NULL");

        // Row 1: real values preserved
        assert_eq!(title.value(1), "hello");
        assert_eq!(count.value(1), 5);
        assert!((score.value(1) - 3.14).abs() < f64::EPSILON);
        assert!(draft.value(1));
    }

    #[test]
    fn chunks_parquet_roundtrip() {
        let dimension = 4;
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

        let batch = build_chunks_batch(&chunks, dimension, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chunks.parquet");

        write_parquet(&path, &batch).unwrap();
        let batches = read_parquet(&path).unwrap();

        assert_eq!(batches.len(), 1);
        let read_batch = &batches[0];
        assert_eq!(read_batch.num_rows(), 3);

        let chunk_ids = read_batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(chunk_ids.value(0), "c1");
        assert_eq!(chunk_ids.value(2), "c3");

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
    }

    #[test]
    fn streaming_write() {
        let schema_fields: Vec<(String, FieldType)> = vec![("title".into(), FieldType::String)];

        let batch1 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "a".into(),
                filename: "a.md".into(),
                frontmatter: Some(serde_json::json!({"title": "First"})),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            }],
            "_",
        );
        let batch2 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "b".into(),
                filename: "b.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Second"})),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            }],
            "_",
        );

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files_streamed.parquet");

        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, batch1.schema(), Some(writer_props())).unwrap();
        writer.write(&batch1).unwrap();
        writer.write(&batch2).unwrap();
        writer.close().unwrap();

        let batches = read_parquet(&path).unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 2);

        let first_batch = &batches[0];
        let filenames = first_batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "a.md");

        let last_batch = batches.last().unwrap();
        let filenames = last_batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(filenames.len() - 1), "b.md");
    }

    #[test]
    fn streaming_read() {
        let schema_fields: Vec<(String, FieldType)> = vec![("title".into(), FieldType::String)];

        let batch1 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "a".into(),
                filename: "a.md".into(),
                frontmatter: Some(serde_json::json!({"title": "First"})),
                content_hash: "h1".into(),
                built_at: 1_700_000_000_000_000,
            }],
            "_",
        );
        let batch2 = build_files_batch(
            &schema_fields,
            &[FileRow {
                file_id: "b".into(),
                filename: "b.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Second"})),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            }],
            "_",
        );

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files_stream_read.parquet");

        let file = File::create(&path).unwrap();
        let mut writer = ArrowWriter::try_new(file, batch1.schema(), Some(writer_props())).unwrap();
        writer.write(&batch1).unwrap();
        writer.write(&batch2).unwrap();
        writer.close().unwrap();

        let file = File::open(&path).unwrap();
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .unwrap()
            .with_batch_size(1)
            .build()
            .unwrap();

        let mut row_count = 0;
        for batch in reader {
            let batch = batch.unwrap();
            row_count += batch.num_rows();
        }
        assert_eq!(row_count, 2);
    }

    #[test]
    fn column_projection() {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
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
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "blog/post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "World"})),
                content_hash: "hash2".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files_proj.parquet");

        write_parquet(&path, &batch).unwrap();

        let file = File::open(&path).unwrap();
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).unwrap();

        // Read only filename and content_hash (columns 1 and 3)
        let mask =
            datafusion::parquet::arrow::ProjectionMask::roots(builder.parquet_schema(), [1, 3]);
        let reader = builder.with_projection(mask).build().unwrap();

        let batches: Vec<RecordBatch> = reader.map(|r| r.unwrap()).collect();
        let batch = &batches[0];
        assert_eq!(batch.num_columns(), 2);
        assert_eq!(batch.schema().field(0).name(), "_filename");
        assert_eq!(batch.schema().field(1).name(), "_content_hash");

        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(filenames.value(0), "blog/post1.md");
    }

    #[test]
    fn file_size_reasonable() {
        let schema_fields = vec![("title".into(), FieldType::String)];

        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"title": "Hello"})),
            content_hash: "hash1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files_size.parquet");

        write_parquet(&path, &batch).unwrap();

        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 0);
        assert!(size < 10_000);
    }

    #[test]
    fn build_metadata_roundtrip() {
        let schema_fields: Vec<(String, FieldType)> = vec![("title".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"title": "Hello"})),
            content_hash: "hash1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");

        let meta = BuildMetadata {
            embedding_model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: Some("abc123".into()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            glob: "**".into(),
            built_at: "2026-03-02T12:00:00+00:00".into(),
            internal_prefix: "_".into(),
        };

        write_parquet_with_metadata(&path, &batch, meta.to_hash_map()).unwrap();

        let read_meta = read_build_metadata(&path).unwrap();
        assert_eq!(read_meta, Some(meta));
    }

    #[test]
    fn build_metadata_no_revision() {
        let schema_fields: Vec<(String, FieldType)> = vec![("title".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"title": "Hello"})),
            content_hash: "hash1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");

        let meta = BuildMetadata {
            embedding_model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 512,
            },
            glob: "blog/**".into(),
            built_at: "2026-03-02T12:00:00+00:00".into(),
            internal_prefix: "_".into(),
        };

        write_parquet_with_metadata(&path, &batch, meta.to_hash_map()).unwrap();

        let read_meta = read_build_metadata(&path).unwrap();
        assert_eq!(read_meta, Some(meta));
    }

    #[test]
    fn read_metadata_missing() {
        let schema_fields: Vec<(String, FieldType)> = vec![("title".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"title": "Hello"})),
            content_hash: "hash1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");

        // Write without metadata
        write_parquet(&path, &batch).unwrap();

        let read_meta = read_build_metadata(&path).unwrap();
        assert_eq!(read_meta, None);
    }

    #[test]
    fn read_file_index_roundtrip() {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
        ];
        let files = vec![
            FileRow {
                file_id: "f1".into(),
                filename: "blog/post1.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Hello", "tags": ["rust"]})),
                content_hash: "abc123".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "f2".into(),
                filename: "blog/post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "World"})),
                content_hash: "def456".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRow {
                file_id: "f3".into(),
                filename: "notes/bare.md".into(),
                frontmatter: None,
                content_hash: "ghi789".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");
        write_parquet(&path, &batch).unwrap();

        let index = read_file_index(&path).unwrap();
        assert_eq!(index.len(), 3);

        assert_eq!(index[0].file_id, "f1");
        assert_eq!(index[0].filename, "blog/post1.md");
        assert_eq!(index[0].content_hash, "abc123");

        assert_eq!(index[1].file_id, "f2");
        assert_eq!(index[1].filename, "blog/post2.md");
        assert_eq!(index[1].content_hash, "def456");

        assert_eq!(index[2].file_id, "f3");
        assert_eq!(index[2].filename, "notes/bare.md");
        assert_eq!(index[2].content_hash, "ghi789");
    }

    #[test]
    fn read_chunk_rows_roundtrip() {
        let original = vec![
            ChunkRow {
                chunk_id: "c1".into(),
                file_id: "f1".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 5,
                embedding: vec![0.1, 0.2, 0.3, 0.4],
            },
            ChunkRow {
                chunk_id: "c2".into(),
                file_id: "f1".into(),
                chunk_index: 1,
                start_line: 7,
                end_line: 12,
                embedding: vec![0.5, 0.6, 0.7, 0.8],
            },
            ChunkRow {
                chunk_id: "c3".into(),
                file_id: "f2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.9, 1.0, 1.1, 1.2],
            },
        ];

        let batch = build_chunks_batch(&original, 4, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("chunks.parquet");
        write_parquet(&path, &batch).unwrap();

        let rows = read_chunk_rows(&path).unwrap();
        assert_eq!(rows.len(), 3);

        assert_eq!(rows[0].chunk_id, "c1");
        assert_eq!(rows[0].file_id, "f1");
        assert_eq!(rows[0].chunk_index, 0);
        assert_eq!(rows[0].start_line, 1);
        assert_eq!(rows[0].end_line, 5);
        assert_eq!(rows[0].embedding.len(), 4);
        assert!((rows[0].embedding[0] - 0.1).abs() < f32::EPSILON);
        assert!((rows[0].embedding[3] - 0.4).abs() < f32::EPSILON);

        assert_eq!(rows[2].chunk_id, "c3");
        assert_eq!(rows[2].file_id, "f2");
        assert!((rows[2].embedding[0] - 0.9).abs() < f32::EPSILON);
    }

    #[test]
    fn nested_object_roundtrip() {
        use std::collections::BTreeMap;

        let mut inner = BTreeMap::new();
        inner.insert("author".into(), FieldType::String);
        inner.insert("version".into(), FieldType::Integer);
        let schema_fields = vec![("meta".into(), FieldType::Object(inner))];

        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "nested.md".into(),
            frontmatter: Some(serde_json::json!({"meta": {"author": "Alice", "version": 2}})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");
        write_parquet(&path, &batch).unwrap();

        let batches = read_parquet(&path).unwrap();
        assert_eq!(batches[0].num_rows(), 1);
    }

    #[test]
    fn unicode_frontmatter_roundtrip() {
        let schema_fields = vec![("title".into(), FieldType::String)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "unicode.md".into(),
            frontmatter: Some(serde_json::json!({"title": "こんにちは 🦀 Émojis"})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");
        write_parquet(&path, &batch).unwrap();

        let batches = read_parquet(&path).unwrap();
        let data_col = batches[0]
            .column_by_name("_data")
            .unwrap()
            .as_any()
            .downcast_ref::<datafusion::arrow::array::StructArray>()
            .unwrap();
        let titles = data_col
            .column_by_name("title")
            .unwrap()
            .as_any()
            .downcast_ref::<datafusion::arrow::array::StringArray>()
            .unwrap();
        assert_eq!(titles.value(0), "こんにちは 🦀 Émojis");
    }

    #[test]
    fn content_hash_determinism() {
        let h1 = content_hash("hello world");
        let h2 = content_hash("hello world");
        let h3 = content_hash("different content");
        assert_eq!(h1, h2, "same content should produce same hash");
        assert_ne!(h1, h3, "different content should produce different hash");
    }

    #[test]
    fn empty_array_roundtrip() {
        let schema_fields = vec![("tags".into(), FieldType::Array(Box::new(FieldType::String)))];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "empty.md".into(),
            frontmatter: Some(serde_json::json!({"tags": []})),
            content_hash: "h1".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files, "_");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("files.parquet");
        write_parquet(&path, &batch).unwrap();

        let batches = read_parquet(&path).unwrap();
        assert_eq!(batches[0].num_rows(), 1);
    }
}
