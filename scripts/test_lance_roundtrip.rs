#!/usr/bin/env rust-script
//! Spike for TODO-0016 (script #1 / storage shape).
//!
//! Proves the `lancedb` crate accepts mdvs's denormalized Arrow schema —
//! a nested `data` Struct column (mirroring dotted leaves like
//! `calibration.baseline.wavelength`) plus an `embedding`
//! `FixedSizeList<Float32, dim>` — and survives a write -> reopen cycle.
//!
//! PASS: reopened row count matches, and a nested struct value + an
//! embedding value read back equal what we wrote.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! arrow-array = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! futures = "0.3"
//! ```

use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Float64Array, Int32Array, RecordBatch,
    RecordBatchIterator, RecordBatchReader, StringArray, StructArray,
};
use arrow_schema::{DataType, Field, Fields, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

const DIM: i32 = 8;

fn build_schema() -> Arc<Schema> {
    // data: { title: Utf8, calibration: { baseline: { wavelength: Float64 } } }
    let wavelength = Field::new("wavelength", DataType::Float64, true);
    let baseline = Field::new(
        "baseline",
        DataType::Struct(Fields::from(vec![wavelength])),
        true,
    );
    let calibration = Field::new(
        "calibration",
        DataType::Struct(Fields::from(vec![baseline])),
        true,
    );
    let data_fields = Fields::from(vec![
        Field::new("title", DataType::Utf8, true),
        calibration,
    ]);

    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("file_id", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int32, false),
        Field::new("data", DataType::Struct(data_fields), true),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
            true,
        ),
    ]))
}

fn build_batch(schema: &Arc<Schema>) -> RecordBatch {
    let n = 3usize;

    let chunk_id = StringArray::from(vec!["c0", "c1", "c2"]);
    let file_id = StringArray::from(vec!["f0", "f0", "f1"]);
    let chunk_index = Int32Array::from(vec![0, 1, 0]);

    // nested data struct
    let titles = StringArray::from(vec!["Dune", "Dune", "Foundation"]);
    let wavelengths = Float64Array::from(vec![Some(850.0), Some(632.8), Some(905.0)]);
    let baseline = StructArray::from(vec![(
        Arc::new(Field::new("wavelength", DataType::Float64, true)),
        Arc::new(wavelengths) as Arc<dyn Array>,
    )]);
    let calibration = StructArray::from(vec![(
        Arc::new(Field::new(
            "baseline",
            baseline.data_type().clone(),
            true,
        )),
        Arc::new(baseline) as Arc<dyn Array>,
    )]);
    let data = StructArray::from(vec![
        (
            Arc::new(Field::new("title", DataType::Utf8, true)),
            Arc::new(titles) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new(
                "calibration",
                calibration.data_type().clone(),
                true,
            )),
            Arc::new(calibration) as Arc<dyn Array>,
        ),
    ]);

    // embeddings: row i = [i.0, i.1, ... i.7]
    let mut flat = Vec::with_capacity(n * DIM as usize);
    for i in 0..n {
        for j in 0..DIM as usize {
            flat.push(i as f32 + (j as f32) / 10.0);
        }
    }
    let values = Float32Array::from(flat);
    let embedding = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        DIM,
        Arc::new(values),
        None,
    );

    RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(chunk_id),
            Arc::new(file_id),
            Arc::new(chunk_index),
            Arc::new(data),
            Arc::new(embedding),
        ],
    )
    .expect("build batch")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("index.lance");
    let uri = uri.to_str().unwrap();

    let schema = build_schema();
    let batch = build_batch(&schema);

    // ---- write ----
    let conn = lancedb::connect(uri).execute().await?;
    let reader = RecordBatchIterator::new(vec![Ok(batch.clone())], schema.clone());
    let reader: Box<dyn RecordBatchReader + Send> = Box::new(reader);
    let table = conn.create_table("index", reader).execute().await?;
    println!("created table, declared rows = {}", batch.num_rows());

    // ---- reopen + read ----
    drop(table);
    drop(conn);
    let conn = lancedb::connect(uri).execute().await?;
    let table = conn.open_table("index").execute().await?;

    let count = table.count_rows(None).await?;
    println!("reopened row count = {count}");
    assert_eq!(count, 3, "row count mismatch after reopen");

    let batches: Vec<RecordBatch> = table
        .query()
        .limit(10)
        .execute()
        .await?
        .try_collect()
        .await?;
    assert!(!batches.is_empty(), "no batches returned");
    let got = &batches[0];
    println!("read back {} rows, {} cols", got.num_rows(), got.num_columns());

    // pull data.calibration.baseline.wavelength for row 0
    let data_col = got
        .column_by_name("data")
        .unwrap()
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let calib = data_col
        .column_by_name("calibration")
        .unwrap()
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let base = calib
        .column_by_name("baseline")
        .unwrap()
        .as_any()
        .downcast_ref::<StructArray>()
        .unwrap();
    let wl = base
        .column_by_name("wavelength")
        .unwrap()
        .as_any()
        .downcast_ref::<Float64Array>()
        .unwrap();
    println!("row0 data.calibration.baseline.wavelength = {}", wl.value(0));
    assert_eq!(wl.value(0), 850.0, "nested struct value mismatch");

    // pull embedding row 0 value 1
    let emb = got
        .column_by_name("embedding")
        .unwrap()
        .as_any()
        .downcast_ref::<FixedSizeListArray>()
        .unwrap();
    let row0 = emb.value(0);
    let row0 = row0.as_any().downcast_ref::<Float32Array>().unwrap();
    println!("row0 embedding[1] = {}", row0.value(1));
    assert!((row0.value(1) - 0.1).abs() < 1e-6, "embedding value mismatch");

    println!("\nPASS: schema round-trips (nested struct + FixedSizeList<Float32>).");
    Ok(())
}
