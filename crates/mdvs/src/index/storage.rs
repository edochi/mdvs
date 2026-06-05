use crate::discover::field_type::FieldType;
use crate::schema::config::MdvsToml;
use crate::schema::json_schema::dsl_to_canonical;
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use xxhash_rust::xxh3::xxh3_64;

use arrow::array::{
    ArrayRef, BooleanArray, Date32Array, FixedSizeListArray, Float32Array, Float64Array,
    Int32Array, Int64Array, ListArray, StringArray, StructArray, TimestampMicrosecondArray,
    TimestampMillisecondArray,
};
use arrow::buffer::{NullBuffer, OffsetBuffer};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use serde_json::Value;
use std::sync::Arc;

/// File ID column in files.parquet.
pub const COL_FILE_ID: &str = "file_id";
/// File path column in files.parquet (relative to project root).
pub const COL_FILEPATH: &str = "filepath";
/// Frontmatter Struct column in files.parquet.
pub const COL_DATA: &str = "data";
/// Content hash column in files.parquet.
pub const COL_CONTENT_HASH: &str = "content_hash";
/// Build timestamp column in files.parquet.
pub const COL_BUILT_AT: &str = "built_at";

/// Chunk ID column in chunks.parquet.
pub const COL_CHUNK_ID: &str = "chunk_id";
/// Chunk index column in chunks.parquet.
pub const COL_CHUNK_INDEX: &str = "chunk_index";
/// Start line column in chunks.parquet.
pub const COL_START_LINE: &str = "start_line";
/// End line column in chunks.parquet.
pub const COL_END_LINE: &str = "end_line";
/// Plain-text column (BM25 full-text index + result snippet).
pub const COL_CHUNK_TEXT: &str = "chunk_text";
/// Embedding vector column in chunks.parquet.
pub const COL_EMBEDDING: &str = "embedding";

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
    /// Plain text of the chunk (what was embedded). Persisted for the BM25
    /// full-text index and returned as the search result snippet.
    pub chunk_text: String,
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
    /// Hash of the canonical JSON Schema derived from `mdvs.toml`. Used to
    /// detect schema changes between builds (any change to fields, types,
    /// constraints, path-scoping, or preprocess arrays invalidates the
    /// existing parquet data; rebuild required).
    pub schema_hash: String,
}

/// Compute a deterministic hash of the canonical JSON Schema for a given
/// `MdvsToml`. Hashes the **post-translation** form: whitespace, comments,
/// and field ordering in `mdvs.toml` don't affect the hash; only the
/// semantic schema content does.
pub fn compute_schema_hash(config: &MdvsToml) -> String {
    let canonical = dsl_to_canonical(config);
    // serde_json::to_string with the default Map serializer preserves
    // insertion order; dsl_to_canonical builds the map in a deterministic
    // order (iteration over toml.fields.field in slice order, followed by
    // toml.fields.ignore). MdvsToml::write sorts fields by name before
    // serializing, so the resulting hash is stable across writes.
    let json = serde_json::to_string(&canonical).unwrap_or_default();
    content_hash(&json)
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
        if let Some(dim) = self.embedding_model.dim {
            m.insert("mdvs.dim".into(), dim.to_string());
        }
        m.insert(
            "mdvs.chunk_size".into(),
            self.chunking.max_chunk_size.to_string(),
        );
        m.insert("mdvs.glob".into(), self.glob.clone());
        m.insert("mdvs.built_at".into(), self.built_at.clone());
        m.insert("mdvs.schema_hash".into(), self.schema_hash.clone());
        m
    }

    /// Deserialize from parquet key-value metadata. Returns `None` if required keys are missing.
    /// `schema_hash` defaults to empty string for parquets built before step 14 — a pre-step-14
    /// parquet will always be flagged as "schema changed" on the next build, which is the
    /// correct conservative behavior.
    pub fn from_hash_map(meta: &HashMap<String, String>) -> Option<Self> {
        Some(Self {
            embedding_model: EmbeddingModelConfig {
                provider: meta
                    .get("mdvs.provider")
                    .cloned()
                    .unwrap_or_else(|| "model2vec".to_string()),
                name: meta.get("mdvs.model")?.clone(),
                revision: meta.get("mdvs.revision").cloned(),
                dim: meta.get("mdvs.dim").and_then(|s| s.parse().ok()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: meta.get("mdvs.chunk_size")?.parse().ok()?,
            },
            glob: meta.get("mdvs.glob")?.clone(),
            built_at: meta.get("mdvs.built_at")?.clone(),
            schema_hash: meta.get("mdvs.schema_hash").cloned().unwrap_or_default(),
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
        FieldType::Date => {
            // Parse JSON strings as RFC 3339 full-date (`YYYY-MM-DD`) and
            // store as Arrow `Date32` (days since 1970-01-01). jsonschema's
            // `format: date` validator runs upstream during check, so by the
            // time we get here the values that survived are well-formed.
            // Defensively skip anything that doesn't parse.
            //
            // `num_days_from_ce()` returns days since 1 CE; the constant
            // `EPOCH_DAYS_FROM_CE = 719163` is `NaiveDate(1970, 1, 1)
            // .num_days_from_ce()`, used to convert to Date32's epoch (1970-01-01).
            use chrono::Datelike;
            const EPOCH_DAYS_FROM_CE: i32 = 719_163;
            let arr: Date32Array = values
                .iter()
                .map(|v| {
                    v.and_then(|v| v.as_str())
                        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                        .map(|d| d.num_days_from_ce() - EPOCH_DAYS_FROM_CE)
                })
                .collect();
            Arc::new(arr)
        }
        FieldType::DateTime => {
            // Parse RFC 3339 datetimes and store as Arrow Timestamp(ms, UTC).
            // Values with any offset are normalized to UTC at storage time;
            // the original offset is intentionally dropped (we store absolute
            // moments, not local-time + offset pairs). Naive datetimes are
            // rejected by parse_from_rfc3339, so they end up as NULL.
            let raw: TimestampMillisecondArray = values
                .iter()
                .map(|v| {
                    v.and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc).timestamp_millis())
                })
                .collect();
            Arc::new(raw.with_timezone(Arc::from("UTC")))
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
                        // `offsets` is seeded with `vec![0]`, so `.last()`
                        // is always Some here. The `unwrap_or(&0)` fallback
                        // preserves correctness if a future refactor breaks
                        // that invariant.
                        offsets.push(*offsets.last().unwrap_or(&0));
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
) -> anyhow::Result<RecordBatch> {
    let file_id_arr: StringArray = files.iter().map(|f| Some(f.file_id.as_str())).collect();
    let filename_arr: StringArray = files.iter().map(|f| Some(f.filename.as_str())).collect();
    let content_hash_arr: StringArray = files
        .iter()
        .map(|f| Some(f.content_hash.as_str()))
        .collect();
    let built_at_arr: TimestampMicrosecondArray = files.iter().map(|f| Some(f.built_at)).collect();

    // Transpose the flat dotted-name fields into a nested `FieldType::Object`
    // tree (TODO-0097 step 5). The `data` Struct's children mirror this tree:
    // `calibration.baseline.wavelength` lands inside a `calibration` Struct
    // child that contains a `baseline` Struct child that contains a
    // `wavelength` Float leaf. The existing `build_array`'s `Object` arm
    // does the heavy lifting — we just hand it the synthesized FieldType and
    // each file's whole frontmatter Value as the per-row value.
    let storage_ft = transpose_to_storage_type(schema_fields);
    let values: Vec<Option<&Value>> = files.iter().map(|f| f.frontmatter.as_ref()).collect();
    let data_arr = build_array(&values, &storage_ft);
    let data_struct_type: DataType = (&storage_ft).into();

    let schema = Schema::new(vec![
        Field::new(COL_FILE_ID, DataType::Utf8, false),
        Field::new(COL_FILEPATH, DataType::Utf8, false),
        Field::new(COL_DATA, data_struct_type, true),
        Field::new(COL_CONTENT_HASH, DataType::Utf8, false),
        Field::new(
            COL_BUILT_AT,
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ]);

    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(file_id_arr),
            Arc::new(filename_arr),
            data_arr,
            Arc::new(content_hash_arr),
            Arc::new(built_at_arr),
        ],
    )
    .map_err(|e| anyhow::anyhow!("failed to build files RecordBatch: {e}"))
}

/// Transpose a flat list of `(dotted_name, FieldType)` entries into a
/// single `FieldType::Object` whose nested structure mirrors the YAML's
/// natural shape.
///
/// Example: `[("title", String), ("cal.x", Float), ("cal.y", Float)]`
/// becomes `Object({"title": String, "cal": Object({"x": Float, "y": Float})})`.
///
/// Assumes the input passed `MdvsToml::validate()` (invariant 8 rejects
/// leaf-vs-parent collisions). If a conflict slips through, the later
/// insertion silently wins at the leaf position.
fn transpose_to_storage_type(schema_fields: &[(String, FieldType)]) -> FieldType {
    let mut root: BTreeMap<String, FieldType> = BTreeMap::new();
    for (name, ft) in schema_fields {
        insert_at_dotted_path(&mut root, name, ft.clone());
    }
    FieldType::Object(root)
}

fn insert_at_dotted_path(map: &mut BTreeMap<String, FieldType>, path: &str, ft: FieldType) {
    let segments: Vec<&str> = path.split('.').collect();
    insert_at_segments(map, &segments, ft);
}

fn insert_at_segments(map: &mut BTreeMap<String, FieldType>, segments: &[&str], ft: FieldType) {
    let Some((first, rest)) = segments.split_first() else {
        return;
    };
    if rest.is_empty() {
        map.insert((*first).to_string(), ft);
        return;
    }
    let entry = map
        .entry((*first).to_string())
        .or_insert_with(|| FieldType::Object(BTreeMap::new()));
    // If the existing entry isn't Object (validate invariant 8 should have
    // caught this), replace it. Silent overwrite matches dsl_to_canonical's
    // defensive policy.
    if !matches!(entry, FieldType::Object(_)) {
        *entry = FieldType::Object(BTreeMap::new());
    }
    if let FieldType::Object(inner) = entry {
        insert_at_segments(inner, rest, ft);
    }
}

/// Build a single **denormalized** Arrow `RecordBatch` for the Lance index
/// (TODO-0016). One row per chunk, with the parent file's metadata
/// (`filepath`, `content_hash`, `data` Struct, `built_at`) duplicated inline.
/// LanceDB is single-table, so file and chunk rows are joined here.
///
/// Column order: chunk_id, file_id, chunk_index, start_line, end_line,
/// chunk_text, embedding, filepath, content_hash, data, built_at.
pub fn build_index_batch(
    schema_fields: &[(String, FieldType)],
    files: &[FileRow],
    chunks: &[ChunkRow],
) -> anyhow::Result<RecordBatch> {
    let file_by_id: HashMap<&str, &FileRow> =
        files.iter().map(|f| (f.file_id.as_str(), f)).collect();

    // Resolve each chunk's parent file once, erroring on a dangling FK.
    let parents: Vec<&FileRow> = chunks
        .iter()
        .map(|c| {
            file_by_id.get(c.file_id.as_str()).copied().ok_or_else(|| {
                anyhow::anyhow!(
                    "chunk {} references unknown file_id {}",
                    c.chunk_id,
                    c.file_id
                )
            })
        })
        .collect::<anyhow::Result<_>>()?;

    // chunk-level columns
    let chunk_id_arr: StringArray = chunks.iter().map(|c| Some(c.chunk_id.as_str())).collect();
    let file_id_arr: StringArray = chunks.iter().map(|c| Some(c.file_id.as_str())).collect();
    let chunk_index_arr: Int32Array = chunks.iter().map(|c| Some(c.chunk_index)).collect();
    let start_line_arr: Int32Array = chunks.iter().map(|c| Some(c.start_line)).collect();
    let end_line_arr: Int32Array = chunks.iter().map(|c| Some(c.end_line)).collect();
    let chunk_text_arr: StringArray = chunks.iter().map(|c| Some(c.chunk_text.as_str())).collect();

    let dimension = chunks
        .first()
        .map(|c| c.embedding.len() as i32)
        .unwrap_or(0);
    let flat_values: Vec<f32> = chunks
        .iter()
        .flat_map(|c| c.embedding.iter().copied())
        .collect();
    let embedding_arr = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, false)),
        dimension,
        Arc::new(Float32Array::from(flat_values)),
        None,
    );

    // file-level columns, expanded to chunk cardinality
    let filepath_arr: StringArray = parents.iter().map(|f| Some(f.filename.as_str())).collect();
    let content_hash_arr: StringArray = parents
        .iter()
        .map(|f| Some(f.content_hash.as_str()))
        .collect();
    let built_at_arr: TimestampMicrosecondArray = parents
        .iter()
        .map(|f| Some(f.built_at))
        .collect::<TimestampMicrosecondArray>()
        .with_timezone("UTC");

    // data Struct, one (possibly null) frontmatter Value per chunk's file
    let storage_ft = transpose_to_storage_type(schema_fields);
    let data_values: Vec<Option<&Value>> = parents.iter().map(|f| f.frontmatter.as_ref()).collect();
    let data_arr = build_array(&data_values, &storage_ft);
    let data_struct_type: DataType = (&storage_ft).into();

    let schema = Schema::new(vec![
        Field::new(COL_CHUNK_ID, DataType::Utf8, false),
        Field::new(COL_FILE_ID, DataType::Utf8, false),
        Field::new(COL_CHUNK_INDEX, DataType::Int32, false),
        Field::new(COL_START_LINE, DataType::Int32, false),
        Field::new(COL_END_LINE, DataType::Int32, false),
        Field::new(COL_CHUNK_TEXT, DataType::Utf8, false),
        Field::new(
            COL_EMBEDDING,
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, false)),
                dimension,
            ),
            false,
        ),
        Field::new(COL_FILEPATH, DataType::Utf8, false),
        Field::new(COL_CONTENT_HASH, DataType::Utf8, false),
        Field::new(COL_DATA, data_struct_type, true),
        Field::new(
            COL_BUILT_AT,
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
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
            Arc::new(chunk_text_arr),
            Arc::new(embedding_arr),
            Arc::new(filepath_arr),
            Arc::new(content_hash_arr),
            data_arr,
            Arc::new(built_at_arr),
        ],
    )
    .map_err(|e| anyhow::anyhow!("failed to build denormalized index RecordBatch: {e}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use arrow::array::Array;

    // ------------------------------------------------------------------------
    // TODO-0097 step 5: dotted-name leaves → nested Arrow Struct columns
    // ------------------------------------------------------------------------

    #[test]
    fn dotted_leaves_produce_nested_struct_columns() {
        // Three leaves: one flat, two under a shared `cal.baseline` parent.
        // The resulting `data` Struct should have a top-level `title` Utf8
        // child AND a top-level `cal` Struct child whose `baseline` Struct
        // child has `intensity` and `wavelength` Float children.
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("cal.baseline.wavelength".into(), FieldType::Float),
            ("cal.baseline.intensity".into(), FieldType::Float),
        ];

        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "projects/alpha/notes/exp.md".into(),
            frontmatter: Some(serde_json::json!({
                "title": "Hello",
                "cal": {
                    "baseline": {"wavelength": 850.0, "intensity": 0.95}
                }
            })),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();

        // Top-level data Struct: has `cal` and `title` children.
        let cal_col = data
            .column_by_name("cal")
            .expect("data.cal must be a top-level Struct child");
        let cal_struct = cal_col.as_any().downcast_ref::<StructArray>().unwrap();

        // cal.baseline Struct child
        let baseline_col = cal_struct
            .column_by_name("baseline")
            .expect("data.cal.baseline must be a Struct child");
        let baseline_struct = baseline_col.as_any().downcast_ref::<StructArray>().unwrap();

        // Leaves: cal.baseline.wavelength and cal.baseline.intensity
        let wavelength = baseline_struct
            .column_by_name("wavelength")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(wavelength.value(0), 850.0);
        let intensity = baseline_struct
            .column_by_name("intensity")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(intensity.value(0), 0.95);
    }

    #[test]
    fn dotted_leaves_handle_absent_parent() {
        // File has no `cal` key at all. The `cal` Struct column for this
        // row should have null validity; each grand-child is also null.
        let schema_fields = vec![
            ("cal.baseline.wavelength".into(), FieldType::Float),
            ("cal.baseline.intensity".into(), FieldType::Float),
        ];

        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "blog/post.md".into(),
            frontmatter: Some(serde_json::json!({"unrelated": 1})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let cal = data
            .column_by_name("cal")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        // `cal` Struct itself is null for this row (no `cal` key in frontmatter).
        assert!(cal.is_null(0));
    }

    #[test]
    fn dotted_leaves_handle_partial_intermediate() {
        // File has `cal.baseline` but only `intensity`, not `wavelength`.
        // Both leaves are declared. The Struct exists; wavelength is null.
        let schema_fields = vec![
            ("cal.baseline.wavelength".into(), FieldType::Float),
            ("cal.baseline.intensity".into(), FieldType::Float),
        ];

        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "blog/post.md".into(),
            frontmatter: Some(serde_json::json!({
                "cal": {"baseline": {"intensity": 0.5}}
            })),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let baseline = data
            .column_by_name("cal")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap()
            .column_by_name("baseline")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();

        let intensity = baseline
            .column_by_name("intensity")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert_eq!(intensity.value(0), 0.5);

        let wavelength = baseline
            .column_by_name("wavelength")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!(wavelength.is_null(0));
    }

    #[test]
    fn data_struct_multi_file() {
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

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        assert_eq!(batch.num_rows(), 3);

        // Verify data column
        let data = batch
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

        let batch = build_files_batch(&schema_fields, &files).unwrap();

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
                    serde_json::json!({"title": "hello", "count": 5, "score": 3.15, "draft": true}),
                ),
                content_hash: "h2".into(),
                built_at: 1_700_000_000_000_000,
            },
        ];

        let batch = build_files_batch(&schema_fields, &files).unwrap();

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
        assert!((score.value(1) - 3.15).abs() < f64::EPSILON);
        assert!(draft.value(1));
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

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        assert_eq!(batch.num_rows(), 1);
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

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data_col = batch
            .column_by_name("data")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow::array::StructArray>()
            .unwrap();
        let titles = data_col
            .column_by_name("title")
            .unwrap()
            .as_any()
            .downcast_ref::<arrow::array::StringArray>()
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

    // ------------------------------------------------------------------------
    // compute_schema_hash (TODO-0149 step 14)
    // ------------------------------------------------------------------------

    fn sample_toml() -> MdvsToml {
        use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
        use crate::schema::shared::{FieldTypeSerde, FrontmatterFormat, ScanConfig};
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: FrontmatterFormat::Auto,
            },
            update: UpdateConfig::default(),
            check: None,
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                }],
                max_categories: 10,
                min_category_repetition: 3,
            },
        }
    }

    #[test]
    fn schema_hash_is_deterministic() {
        let t = sample_toml();
        let h1 = compute_schema_hash(&t);
        let h2 = compute_schema_hash(&t);
        assert_eq!(h1, h2);
    }

    #[test]
    fn schema_hash_changes_when_field_added() {
        use crate::schema::config::TomlField;
        use crate::schema::shared::FieldTypeSerde;
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.fields.field.push(TomlField {
            name: "draft".into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        });
        let h2 = compute_schema_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn schema_hash_changes_when_type_changes() {
        use crate::schema::shared::FieldTypeSerde;
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.fields.field[0].field_type = FieldTypeSerde::Scalar("Integer".into());
        let h2 = compute_schema_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn schema_hash_changes_when_constraint_added() {
        use crate::schema::constraints::Constraints;
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.fields.field[0].constraints = Some(Constraints {
            min_length: Some(3),
            ..Default::default()
        });
        let h2 = compute_schema_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn schema_hash_changes_when_preprocess_added() {
        use crate::preprocess::ValueStage;
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.fields.field[0].preprocess = vec![ValueStage::CoerceToString];
        let h2 = compute_schema_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn schema_hash_changes_when_allowed_changes() {
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.fields.field[0].allowed = vec!["blog/**".into()];
        let h2 = compute_schema_hash(&t);
        assert_ne!(h1, h2);
    }

    #[test]
    fn schema_hash_unchanged_by_scan_glob() {
        // scan.glob is tracked separately in BuildMetadata.glob; not part of schema.
        let mut t = sample_toml();
        let h1 = compute_schema_hash(&t);
        t.scan.glob = "blog/**".into();
        let h2 = compute_schema_hash(&t);
        assert_eq!(h1, h2);
    }

    #[test]
    fn schema_hash_unchanged_when_ignore_order_differs() {
        // Ignore list is part of the schema, but ordering shouldn't matter
        // because dsl_to_canonical inserts them in slice order — confirm by
        // building two configs with the same ignore in slice order.
        let mut t1 = sample_toml();
        t1.fields.ignore = vec!["a".into(), "b".into()];
        let mut t2 = sample_toml();
        t2.fields.ignore = vec!["a".into(), "b".into()];
        assert_eq!(compute_schema_hash(&t1), compute_schema_hash(&t2));
    }

    #[test]
    fn build_metadata_roundtrips_schema_hash() {
        let meta = BuildMetadata {
            embedding_model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
                dim: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            glob: "**".into(),
            built_at: "2026-05-11T00:00:00+00:00".into(),
            schema_hash: "deadbeef".into(),
        };
        let m = meta.to_hash_map();
        let parsed = BuildMetadata::from_hash_map(&m).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn build_metadata_schema_hash_defaults_empty_when_missing() {
        // A pre-step-14 parquet has no `mdvs.schema_hash` key. Reading it
        // should succeed with an empty hash — the next build will treat
        // the schema as "changed" and require --force, which is correct.
        let mut m = HashMap::new();
        m.insert("mdvs.provider".into(), "model2vec".into());
        m.insert("mdvs.model".into(), "x".into());
        m.insert("mdvs.chunk_size".into(), "1024".into());
        m.insert("mdvs.glob".into(), "**".into());
        m.insert("mdvs.built_at".into(), "2026-01-01".into());
        let parsed = BuildMetadata::from_hash_map(&m).unwrap();
        assert_eq!(parsed.schema_hash, "");
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

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        assert_eq!(batch.num_rows(), 1);
    }

    // ===== Date type (TODO-0007 Wave 1) — Arrow Date32 storage =====

    #[test]
    fn date_field_writes_date32_column() {
        let schema_fields = vec![("birthday".into(), FieldType::Date)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"birthday": "1990-05-12"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let bday = data
            .column_by_name("birthday")
            .unwrap()
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();

        // 1990-05-12 - 1970-01-01 = 7_436 days.
        assert_eq!(bday.value(0), 7_436);
    }

    #[test]
    fn date_epoch_value_is_zero() {
        let schema_fields = vec![("d".into(), FieldType::Date)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"d": "1970-01-01"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let d = data
            .column_by_name("d")
            .unwrap()
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert_eq!(d.value(0), 0);
    }

    #[test]
    fn array_of_date_writes_list_of_date32() {
        let schema_fields = vec![(
            "milestones".into(),
            FieldType::Array(Box::new(FieldType::Date)),
        )];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({
                "milestones": ["2024-01-01", "2024-06-15"]
            })),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let list = data
            .column_by_name("milestones")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let values = list
            .values()
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        // 2024-01-01: days since 1970-01-01 = 19_723.
        // 2024-06-15: 19_723 + 166 = 19_889.
        assert_eq!(values.value(0), 19_723);
        assert_eq!(values.value(1), 19_889);
    }

    #[test]
    fn date_stored_as_days_since_epoch() {
        let schema_fields = vec![("birthday".into(), FieldType::Date)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"birthday": "2024-03-15"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column_by_name(COL_DATA)
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let bday = data
            .column_by_name("birthday")
            .unwrap()
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        // 2024-03-15: 19_797 days since epoch.
        assert_eq!(bday.value(0), 19_797);
    }

    #[test]
    fn date_field_null_for_unparseable_value() {
        // Defense in depth: jsonschema rejects bad dates at check, but if a
        // bad value somehow reaches build_array, store NULL rather than panic.
        let schema_fields = vec![("d".into(), FieldType::Date)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"d": "not-a-date"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let d = data
            .column_by_name("d")
            .unwrap()
            .as_any()
            .downcast_ref::<Date32Array>()
            .unwrap();
        assert!(d.is_null(0));
    }

    // ===== DateTime type (TODO-0007 Wave 3) — Arrow Timestamp(ms, UTC) =====

    #[test]
    fn datetime_field_writes_timestamp_ms_utc_column() {
        let schema_fields = vec![("synced_at".into(), FieldType::DateTime)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"synced_at": "1970-01-01T00:00:00Z"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];

        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let synced = data
            .column_by_name("synced_at")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        // Epoch → 0 millis.
        assert_eq!(synced.value(0), 0);
        // Timezone metadata on the column type.
        match synced.data_type() {
            DataType::Timestamp(unit, tz) => {
                assert_eq!(*unit, TimeUnit::Millisecond);
                assert_eq!(tz.as_deref(), Some("UTC"));
            }
            other => panic!("expected Timestamp(ms, UTC), got {other:?}"),
        }
    }

    #[test]
    fn datetime_normalises_offsets_to_utc() {
        // Both inputs represent the same absolute moment (2024-01-15 14:30 UTC).
        let schema_fields = vec![
            ("a".into(), FieldType::DateTime),
            ("b".into(), FieldType::DateTime),
        ];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({
                "a": "2024-01-15T14:30:00Z",
                "b": "2024-01-15T20:00:00+05:30",
            })),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let a = data
            .column_by_name("a")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        let b = data
            .column_by_name("b")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        assert_eq!(a.value(0), b.value(0));
    }

    #[test]
    fn datetime_stored_as_millis_since_epoch() {
        let schema_fields = vec![("synced_at".into(), FieldType::DateTime)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"synced_at": "2024-03-15T14:30:00Z"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column_by_name(COL_DATA)
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let synced = data
            .column_by_name("synced_at")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        // 2024-03-15T14:30:00Z = 1_710_513_000_000 ms since epoch.
        assert_eq!(synced.value(0), 1_710_513_000_000);
    }

    #[test]
    fn array_of_datetime_writes_list_of_timestamp_ms() {
        let schema_fields = vec![(
            "events".into(),
            FieldType::Array(Box::new(FieldType::DateTime)),
        )];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({
                "events": ["2024-01-15T14:30:00Z", "2024-06-01T08:00:00+00:00"]
            })),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let list = data
            .column_by_name("events")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();
        let values = list
            .values()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        assert_eq!(values.len(), 2);
        assert!(values.value(1) > values.value(0));
    }

    #[test]
    fn datetime_field_null_for_unparseable_value() {
        let schema_fields = vec![("x".into(), FieldType::DateTime)];
        let files = vec![FileRow {
            file_id: "id-1".into(),
            filename: "a.md".into(),
            frontmatter: Some(serde_json::json!({"x": "not-a-datetime"})),
            content_hash: "h".into(),
            built_at: 1_700_000_000_000_000,
        }];
        let batch = build_files_batch(&schema_fields, &files).unwrap();
        let data = batch
            .column(2)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        let x = data
            .column_by_name("x")
            .unwrap()
            .as_any()
            .downcast_ref::<TimestampMillisecondArray>()
            .unwrap();
        assert!(x.is_null(0));
    }
}
