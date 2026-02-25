use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{
    Array, FixedSizeListArray, Float32Array, Float32Builder, FixedSizeListBuilder, Int32Array,
    Int32Builder, RecordBatch, StringBuilder, TimestampMicrosecondArray,
    TimestampMicrosecondBuilder,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

#[derive(Debug, Clone, PartialEq)]
pub struct FileRecord {
    pub file_id: String,
    pub filename: String,
    pub frontmatter: Option<String>,
    pub content_hash: String,
    pub built_at: i64, // microseconds since epoch
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChunkRecord {
    pub chunk_id: String,
    pub file_id: String,
    pub chunk_index: i32,
    pub start_line: i32,
    pub end_line: i32,
    pub embedding: Vec<f32>,
}

fn files_schema() -> Schema {
    Schema::new(vec![
        Field::new("file_id", DataType::Utf8, false),
        Field::new("filename", DataType::Utf8, false),
        Field::new("frontmatter", DataType::Utf8, true),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new(
            "built_at",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ])
}

fn chunks_schema(dimension: usize) -> Schema {
    Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("file_id", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("start_line", DataType::Int32, false),
        Field::new("end_line", DataType::Int32, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimension as i32,
            ),
            false,
        ),
    ])
}

fn writer_props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build()
}

/// Write files.parquet. Overwrites if exists.
pub fn write_files(path: &Path, records: &[FileRecord]) -> Result<()> {
    let schema = Arc::new(files_schema());

    let mut file_id = StringBuilder::new();
    let mut filename = StringBuilder::new();
    let mut frontmatter = StringBuilder::new();
    let mut content_hash = StringBuilder::new();
    let mut built_at = TimestampMicrosecondBuilder::new();

    for r in records {
        file_id.append_value(&r.file_id);
        filename.append_value(&r.filename);
        match &r.frontmatter {
            Some(fm) => frontmatter.append_value(fm),
            None => frontmatter.append_null(),
        }
        content_hash.append_value(&r.content_hash);
        built_at.append_value(r.built_at);
    }

    let batch = RecordBatch::try_new(schema.clone(), vec![
        Arc::new(file_id.finish()),
        Arc::new(filename.finish()),
        Arc::new(frontmatter.finish()),
        Arc::new(content_hash.finish()),
        Arc::new(built_at.finish()),
    ])?;

    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// Write chunks.parquet. Overwrites if exists.
pub fn write_chunks(path: &Path, records: &[ChunkRecord], dimension: usize) -> Result<()> {
    let schema = Arc::new(chunks_schema(dimension));

    let mut chunk_id = StringBuilder::new();
    let mut file_id = StringBuilder::new();
    let mut chunk_index = Int32Builder::new();
    let mut start_line = Int32Builder::new();
    let mut end_line = Int32Builder::new();
    let mut embedding =
        FixedSizeListBuilder::new(Float32Builder::new(), dimension as i32);

    for r in records {
        chunk_id.append_value(&r.chunk_id);
        file_id.append_value(&r.file_id);
        chunk_index.append_value(r.chunk_index);
        start_line.append_value(r.start_line);
        end_line.append_value(r.end_line);

        let values = embedding.values();
        for &v in &r.embedding {
            values.append_value(v);
        }
        embedding.append(true);
    }

    let batch = RecordBatch::try_new(schema.clone(), vec![
        Arc::new(chunk_id.finish()),
        Arc::new(file_id.finish()),
        Arc::new(chunk_index.finish()),
        Arc::new(start_line.finish()),
        Arc::new(end_line.finish()),
        Arc::new(embedding.finish()),
    ])?;

    let file = File::create(path).with_context(|| format!("creating {}", path.display()))?;
    let mut writer = ArrowWriter::try_new(file, schema, Some(writer_props()))?;
    writer.write(&batch)?;
    writer.close()?;
    Ok(())
}

/// Read all file records from files.parquet.
pub fn read_files(path: &Path) -> Result<Vec<FileRecord>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;

    let mut records = Vec::new();
    for batch in reader {
        let batch = batch?;
        let file_ids = batch.column(0).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let filenames = batch.column(1).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let frontmatters = batch.column(2).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let content_hashes = batch.column(3).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let built_ats = batch.column(4).as_any().downcast_ref::<TimestampMicrosecondArray>().unwrap();

        for i in 0..batch.num_rows() {
            records.push(FileRecord {
                file_id: file_ids.value(i).to_string(),
                filename: filenames.value(i).to_string(),
                frontmatter: if frontmatters.is_null(i) {
                    None
                } else {
                    Some(frontmatters.value(i).to_string())
                },
                content_hash: content_hashes.value(i).to_string(),
                built_at: built_ats.value(i),
            });
        }
    }
    Ok(records)
}

/// Read all chunk records from chunks.parquet.
pub fn read_chunks(path: &Path) -> Result<Vec<ChunkRecord>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let reader = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;

    let mut records = Vec::new();
    for batch in reader {
        let batch = batch?;
        let chunk_ids = batch.column(0).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let file_ids = batch.column(1).as_any().downcast_ref::<arrow::array::StringArray>().unwrap();
        let chunk_indices = batch.column(2).as_any().downcast_ref::<Int32Array>().unwrap();
        let start_lines = batch.column(3).as_any().downcast_ref::<Int32Array>().unwrap();
        let end_lines = batch.column(4).as_any().downcast_ref::<Int32Array>().unwrap();
        let embeddings = batch.column(5).as_any().downcast_ref::<FixedSizeListArray>().unwrap();

        for i in 0..batch.num_rows() {
            let emb_array = embeddings.value(i);
            let floats = emb_array.as_any().downcast_ref::<Float32Array>().unwrap();
            let embedding = floats.values().to_vec();

            records.push(ChunkRecord {
                chunk_id: chunk_ids.value(i).to_string(),
                file_id: file_ids.value(i).to_string(),
                chunk_index: chunk_indices.value(i),
                start_line: start_lines.value(i),
                end_line: end_lines.value(i),
                embedding,
            });
        }
    }
    Ok(records)
}

/// Load chunks.parquet as a single Arrow RecordBatch for cosine distance computation.
pub fn load_chunks_batch(path: &Path) -> Result<RecordBatch> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let schema = builder.schema().clone();
    let reader = builder.build()?;

    let batches: Vec<RecordBatch> = reader.collect::<std::result::Result<_, _>>()?;
    if batches.is_empty() {
        return Ok(RecordBatch::new_empty(schema));
    }
    Ok(arrow::compute::concat_batches(&schema, &batches)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_file_records() -> Vec<FileRecord> {
        vec![
            FileRecord {
                file_id: "f1".into(),
                filename: "notes/hello.md".into(),
                frontmatter: Some(r#"{"title":"Hello"}"#.into()),
                content_hash: "abc123".into(),
                built_at: 1_700_000_000_000_000,
            },
            FileRecord {
                file_id: "f2".into(),
                filename: "notes/world.md".into(),
                frontmatter: None,
                content_hash: "def456".into(),
                built_at: 1_700_000_001_000_000,
            },
        ]
    }

    fn sample_chunk_records() -> Vec<ChunkRecord> {
        vec![
            ChunkRecord {
                chunk_id: "c1".into(),
                file_id: "f1".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 5,
                embedding: vec![0.1, 0.2, 0.3],
            },
            ChunkRecord {
                chunk_id: "c2".into(),
                file_id: "f1".into(),
                chunk_index: 1,
                start_line: 6,
                end_line: 10,
                embedding: vec![0.4, 0.5, 0.6],
            },
            ChunkRecord {
                chunk_id: "c3".into(),
                file_id: "f2".into(),
                chunk_index: 0,
                start_line: 1,
                end_line: 3,
                embedding: vec![0.7, 0.8, 0.9],
            },
        ]
    }

    #[test]
    fn round_trip_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.parquet");
        let records = sample_file_records();

        write_files(&path, &records).unwrap();
        let read_back = read_files(&path).unwrap();
        assert_eq!(records, read_back);
    }

    #[test]
    fn round_trip_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chunks.parquet");
        let records = sample_chunk_records();

        write_chunks(&path, &records, 3).unwrap();
        let read_back = read_chunks(&path).unwrap();
        assert_eq!(records, read_back);
    }

    #[test]
    fn empty_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.parquet");

        write_files(&path, &[]).unwrap();
        let read_back = read_files(&path).unwrap();
        assert!(read_back.is_empty());
    }

    #[test]
    fn empty_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chunks.parquet");

        write_chunks(&path, &[], 3).unwrap();
        let read_back = read_chunks(&path).unwrap();
        assert!(read_back.is_empty());
    }

    #[test]
    fn load_chunks_batch_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("chunks.parquet");
        let records = sample_chunk_records();

        write_chunks(&path, &records, 3).unwrap();
        let batch = load_chunks_batch(&path).unwrap();

        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 6);

        // Verify embedding column is FixedSizeList<Float32>(3)
        let schema = batch.schema();
        let emb_field = schema.field(5);
        assert_eq!(emb_field.name(), "embedding");
        match emb_field.data_type() {
            DataType::FixedSizeList(_, size) => assert_eq!(*size, 3),
            other => panic!("expected FixedSizeList, got {other:?}"),
        }
    }

    #[test]
    fn nullable_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("files.parquet");

        let records = vec![
            FileRecord {
                file_id: "f1".into(),
                filename: "with_fm.md".into(),
                frontmatter: Some(r#"{"tags":["a"]}"#.into()),
                content_hash: "aaa".into(),
                built_at: 1_000_000,
            },
            FileRecord {
                file_id: "f2".into(),
                filename: "bare.md".into(),
                frontmatter: None,
                content_hash: "bbb".into(),
                built_at: 2_000_000,
            },
        ];

        write_files(&path, &records).unwrap();
        let read_back = read_files(&path).unwrap();
        assert_eq!(read_back[0].frontmatter, Some(r#"{"tags":["a"]}"#.into()));
        assert_eq!(read_back[1].frontmatter, None);
    }
}
