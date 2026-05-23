#!/usr/bin/env rust-script
//! Bisection spike for TODO-0159. Hypothesis: the panic needs a *wide* data
//! struct (example_kb has 46 children in the `data` Struct, my prior spikes
//! had 2). Builds a synthetic table whose `data` Struct width is sweepable,
//! with a sparse `measurement_values: List<Float64>` child, plus chunk_text
//! and embedding columns and an FTS index. Then runs a hybrid query with
//! `data.measurement_values IS NOT NULL` and watches for a panic / hang.
//!
//! Each width is run inside `tokio::time::timeout` so a worker-thread panic
//! (which would otherwise hang the script) is reported as a TIMEOUT.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! lance-index = "=6.0.0"
//! arrow = "58"
//! arrow-array = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! futures = "0.3"
//! anyhow = "1"
//! ```

use std::sync::Arc;
use std::time::Duration;

use arrow::record_batch::RecordBatch;
use arrow_array::builder::{Float64Builder, ListBuilder, StringBuilder, StructBuilder};
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatchIterator, RecordBatchReader,
    StructArray,
};
use arrow_schema::{DataType, Field, Fields, Schema};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase, Select};

const DIM: i32 = 8;
const N_ROWS: usize = 60;
const N_WITH_LISTS: usize = 2;

/// Build the schema with `n_padding_fields` extra `Utf8` children in `data`
/// alongside the sparse `measurement_values: List<Float64>`.
fn build_schema(n_padding_fields: usize) -> (Arc<Schema>, Fields) {
    let mut data_fields: Vec<Field> = Vec::new();
    for i in 0..n_padding_fields {
        data_fields.push(Field::new(format!("pad_{i:03}"), DataType::Utf8, true));
    }
    data_fields.push(Field::new(
        "measurement_values",
        DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
        true,
    ));
    let data_fields = Fields::from(data_fields);

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("data", DataType::Struct(data_fields.clone()), true),
        Field::new("chunk_text", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
            true,
        ),
    ]));
    (schema, data_fields)
}

fn build_batch(schema: &Arc<Schema>, data_fields: &Fields) -> RecordBatch {
    // ids
    let id = Int32Array::from((0..N_ROWS as i32).collect::<Vec<_>>());

    // chunk_text: real-ish English so the FTS index has multi-term postings
    let chunk_text = arrow_array::StringArray::from((0..N_ROWS)
        .map(|i| {
            format!(
                "experiment {i} calibration drift sensor wavelength baseline measurement \
                 photonics chip array data acquisition pipeline notes from session number {i}"
            )
        })
        .collect::<Vec<_>>());

    // 8-d unit-ish embeddings (round-robin axes)
    let mut emb_vals: Vec<f32> = Vec::with_capacity(N_ROWS * DIM as usize);
    for i in 0..N_ROWS {
        for j in 0..DIM as usize {
            emb_vals.push(if i % DIM as usize == j { 1.0 } else { 0.0 });
        }
    }
    let embedding = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        DIM,
        Arc::new(Float32Array::from(emb_vals)),
        None,
    );

    // data struct: every padding field null on every row; measurement_values
    // present on rows 0 and 1 with different lengths (matching example_kb).
    let mut field_builders: Vec<Box<dyn arrow_array::builder::ArrayBuilder>> =
        Vec::with_capacity(data_fields.len());
    for field in data_fields.iter() {
        if field.name() == "measurement_values" {
            field_builders.push(Box::new(ListBuilder::new(Float64Builder::new())));
        } else {
            field_builders.push(Box::new(StringBuilder::new()));
        }
    }
    let mut sb = StructBuilder::new(data_fields.clone(), field_builders);

    let n_pad = data_fields.len() - 1;
    let mv_idx = n_pad; // last field is measurement_values

    for i in 0..N_ROWS {
        for p in 0..n_pad {
            sb.field_builder::<StringBuilder>(p).unwrap().append_null();
        }
        let mv = sb
            .field_builder::<ListBuilder<Float64Builder>>(mv_idx)
            .unwrap();
        if i == 0 {
            for v in [0.847, 0.853, 0.851] {
                mv.values().append_value(v);
            }
            mv.append(true);
        } else if i == 1 {
            for v in [0.612, 0.598] {
                mv.values().append_value(v);
            }
            mv.append(true);
        } else {
            mv.append(false); // null list
        }
        sb.append(true);
    }
    let data: StructArray = sb.finish();

    RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id),
            Arc::new(data),
            Arc::new(chunk_text),
            Arc::new(embedding),
        ],
    )
    .unwrap()
}

async fn run_one(n_pad: usize) -> &'static str {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return "tempdir-failed",
    };
    let uri = match dir.path().join("t.lance").to_str() {
        Some(s) => s.to_string(),
        None => return "non-utf8",
    };

    let (schema, data_fields) = build_schema(n_pad);
    let batch = build_batch(&schema, &data_fields);
    let _ = N_WITH_LISTS;

    let conn = match lancedb::connect(&uri).execute().await {
        Ok(c) => c,
        Err(_) => return "connect-failed",
    };
    let reader: Box<dyn RecordBatchReader + Send> = Box::new(RecordBatchIterator::new(
        vec![Ok(batch)],
        schema.clone(),
    ));
    let table = match conn.create_table("t", reader).execute().await {
        Ok(t) => t,
        Err(_) => return "create-failed",
    };
    if (table
        .create_index(&["chunk_text"], Index::FTS(Default::default()))
        .execute()
        .await)
        .is_err()
    {
        return "fts-index-failed";
    }

    // hybrid query (nearest_to + full_text_search + only_if on the sparse
    // float list). This is what panics on example_kb.
    let q = table
        .query()
        .nearest_to(&[1.0f32, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let q = match q {
        Ok(q) => q,
        Err(_) => return "nearest_to-failed",
    };
    // Mirror mdvs's narrow projection: exclude `data` and `embedding`.
    let q = q
        .select(Select::columns(&["id", "chunk_text"]))
        .distance_type(lancedb::DistanceType::Cosine)
        .full_text_search(FullTextSearchQuery::new("calibration".to_string()))
        .only_if("data.measurement_values IS NOT NULL")
        .limit(20);
    let stream_res = q.execute().await;
    let stream = match stream_res {
        Ok(s) => s,
        Err(_) => return "execute-failed",
    };
    let collected: Result<Vec<RecordBatch>, _> = stream.try_collect().await;
    match collected {
        Ok(batches) => {
            let n: usize = batches.iter().map(|b| b.num_rows()).sum();
            if n == 2 { "OK(2)" } else { "OK(?)" }
        }
        Err(_) => "stream-err",
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Sweeping `data` struct width with sparse List<Float64> child +");
    println!("hybrid query + filter on it. {N_ROWS} rows, lists in 2 rows.\n");
    for n_pad in [1usize, 4, 16, 32, 45, 80, 200] {
        print!("  pad_fields = {n_pad:4}  ");
        match tokio::time::timeout(Duration::from_secs(30), run_one(n_pad)).await {
            Ok(v) => println!("→ {v}"),
            Err(_) => println!("→ TIMEOUT (likely worker-thread panic = TODO-0159)"),
        }
    }
    println!(
        "\nIf any row says TIMEOUT or this script aborted with a panic stack,\nthe minimal reproducer is at that width."
    );
    Ok(())
}
