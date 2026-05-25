#!/usr/bin/env rust-script
//! Side-by-side: filtering a `List<Float64>` column through DataFusion +
//! Parquet (the pre-swap engine) vs. LanceDB (TODO-0016 wave 2).
//!
//! Same in-memory `RecordBatch` is written both to a Parquet file and to a
//! Lance table, then the same predicates are evaluated against each. Three
//! Lance modes are exercised:
//!  - plain `only_if` scan,
//!  - `nearest_to` (vector) + `only_if` (mirrors `mdvs search --mode semantic`),
//!  - `nearest_to` + `full_text_search` + `only_if` (mirrors `--mode hybrid`).
//!
//! TODO-0159 background: in mdvs's full setup on example_kb, any `--where`
//! touching an `Array(Float)` (`List<Float64>`) field panics inside
//! `lance-encoding 6.0::FixedFullZipDecoder::slice_next_task` — and in
//! hybrid/fulltext the panic lands on a tokio worker, so the main task
//! hangs. This script can't *always* reproduce the panic (the bug needs
//! example_kb's full table shape — wide struct, FTS index, varying list
//! lengths at scale), but it cleanly demonstrates what DataFusion+Parquet
//! does with the same predicates as a reference point.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! lance-index = "=6.0.0"
//! datafusion = { version = "53", default-features = false, features = ["parquet", "sql", "nested_expressions"] }
//! arrow = "58"
//! arrow-array = "58"
//! arrow-schema = "58"
//! parquet = "58"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! futures = "0.3"
//! anyhow = "1"
//! ```

use std::sync::Arc;

use arrow::record_batch::RecordBatch;
use arrow_array::builder::{Float32Builder, Float64Builder, ListBuilder};
use arrow_array::{
    Array, FixedSizeListArray, Int32Array, Int64Array, RecordBatchIterator, RecordBatchReader,
};
use arrow_schema::{DataType, Field, Schema};
use datafusion::prelude::{ParquetReadOptions, SessionContext};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase};

const DIM: i32 = 4;
const N: usize = 6;

fn build_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new(
            "arr",
            DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
            true,
        ),
        Field::new("chunk_text", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
            true,
        ),
    ]))
}

fn build_batch(schema: &Arc<Schema>) -> RecordBatch {
    // rows 0,1: arr has values (different lengths, like example_kb's
    // measurement_values: [0.847, 0.853, 0.851] and [0.612, 0.598]).
    // rows 2..N: arr null.
    let ids = Int32Array::from((0..N as i32).collect::<Vec<_>>());

    let mut lb = ListBuilder::new(Float64Builder::new());
    for v in [0.847, 0.853, 0.851] {
        lb.values().append_value(v);
    }
    lb.append(true);
    for v in [0.612, 0.598] {
        lb.values().append_value(v);
    }
    lb.append(true);
    for _ in 0..(N - 2) {
        lb.append(false);
    }
    let arr = lb.finish();

    let chunk_text = arrow_array::StringArray::from(vec![
        "alpha calibration text",
        "beta drift text",
        "gamma sensor text",
        "delta wavelength text",
        "epsilon photonics text",
        "zeta metamaterial text",
    ]);

    // simple distinct unit vectors (4-d) so nearest_to has something to do
    let mut fb = Float32Builder::new();
    for i in 0..N {
        for j in 0..DIM as usize {
            fb.append_value(if i % DIM as usize == j { 1.0 } else { 0.0 });
        }
    }
    let embedding = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        DIM,
        Arc::new(fb.finish()),
        None,
    );

    RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(ids), Arc::new(arr), Arc::new(chunk_text), Arc::new(embedding)],
    )
    .unwrap()
}

async fn datafusion_count(ctx: &SessionContext, sql: &str) -> anyhow::Result<i64> {
    let df = ctx.sql(sql).await.map_err(anyhow_from)?;
    let batches = df.collect().await.map_err(anyhow_from)?;
    let n = batches[0]
        .column(0)
        .as_any()
        .downcast_ref::<Int64Array>()
        .ok_or_else(|| anyhow::anyhow!("expected Int64Array for count"))?
        .value(0);
    Ok(n)
}

async fn lance_count_plain(table: &lancedb::table::Table, filter: &str) -> anyhow::Result<usize> {
    let batches: Vec<RecordBatch> = table
        .query()
        .only_if(filter)
        .execute()
        .await?
        .try_collect()
        .await?;
    Ok(batches.iter().map(|b| b.num_rows()).sum())
}

async fn lance_count_semantic(table: &lancedb::table::Table, filter: &str) -> anyhow::Result<usize> {
    let batches: Vec<RecordBatch> = table
        .query()
        .nearest_to(&[1.0, 0.0, 0.0, 0.0])?
        .distance_type(lancedb::DistanceType::Cosine)
        .only_if(filter)
        .limit(20)
        .execute()
        .await?
        .try_collect()
        .await?;
    Ok(batches.iter().map(|b| b.num_rows()).sum())
}

async fn lance_count_hybrid(table: &lancedb::table::Table, filter: &str) -> anyhow::Result<usize> {
    let batches: Vec<RecordBatch> = table
        .query()
        .nearest_to(&[1.0, 0.0, 0.0, 0.0])?
        .distance_type(lancedb::DistanceType::Cosine)
        .full_text_search(FullTextSearchQuery::new("text".to_string()))
        .only_if(filter)
        .limit(20)
        .execute()
        .await?
        .try_collect()
        .await?;
    Ok(batches.iter().map(|b| b.num_rows()).sum())
}

fn anyhow_from<E: std::fmt::Display>(e: E) -> anyhow::Error {
    anyhow::anyhow!("{e}")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let pq_path = dir.path().join("t.parquet");
    let lance_db = dir.path().join("lance_db");

    let schema = build_schema();
    let batch = build_batch(&schema);
    println!("Built dataset: {} rows, 'arr' (List<Float64>) non-null in 2 rows.\n", N);

    // ─── DataFusion + Parquet ───────────────────────────────────────────────
    println!("─── DataFusion + Parquet (the pre-swap engine) ───");
    {
        let file = std::fs::File::create(&pq_path)?;
        let mut writer = parquet::arrow::ArrowWriter::try_new(file, schema.clone(), None)?;
        writer.write(&batch)?;
        writer.close()?;
    }
    let ctx = SessionContext::new();
    ctx.register_parquet("t", pq_path.to_str().unwrap(), ParquetReadOptions::default())
        .await
        .map_err(anyhow_from)?;

    for sql in [
        "SELECT count(*) FROM t WHERE arr IS NOT NULL",
        "SELECT count(*) FROM t WHERE arr IS NULL",
        "SELECT count(*) FROM t WHERE array_has(arr, 0.847)",
    ] {
        let n = datafusion_count(&ctx, sql).await?;
        println!("  {sql:60} → n={n}");
    }

    // ─── LanceDB ────────────────────────────────────────────────────────────
    println!("\n─── LanceDB 0.29 (the new engine) ───");
    let conn = lancedb::connect(lance_db.to_str().unwrap())
        .execute()
        .await?;
    let reader: Box<dyn RecordBatchReader + Send> = Box::new(RecordBatchIterator::new(
        vec![Ok(batch.clone())],
        schema.clone(),
    ));
    let table = conn.create_table("t", reader).execute().await?;
    // FTS index, like mdvs does, so the hybrid path is realistic
    table
        .create_index(&["chunk_text"], Index::FTS(Default::default()))
        .execute()
        .await?;

    // Each test below catches its own error so one bad filter doesn't stop
    // the others. A worker-thread panic in the hybrid path would still abort
    // (or hang) — that's the demonstration.
    println!("  [plain `only_if` scan]");
    for f in ["arr IS NOT NULL", "arr IS NULL", "array_has(arr, 0.847)"] {
        match lance_count_plain(&table, f).await {
            Ok(n) => println!("    {f:50} → n={n}"),
            Err(e) => println!("    {f:50} → ERR: {}", first_line(&e.to_string())),
        }
    }

    println!("  [`nearest_to` + `only_if`  (≈ --mode semantic)]");
    for f in ["arr IS NOT NULL", "array_has(arr, 0.847)"] {
        match lance_count_semantic(&table, f).await {
            Ok(n) => println!("    {f:50} → n={n}"),
            Err(e) => println!("    {f:50} → ERR: {}", first_line(&e.to_string())),
        }
    }

    println!("  [`nearest_to` + `full_text_search` + `only_if`  (≈ --mode hybrid)]");
    for f in ["arr IS NOT NULL", "array_has(arr, 0.847)"] {
        match lance_count_hybrid(&table, f).await {
            Ok(n) => println!("    {f:50} → n={n}"),
            Err(e) => println!("    {f:50} → ERR: {}", first_line(&e.to_string())),
        }
    }

    println!(
        "\nIf the Lance hybrid/semantic sections printed answers, the bug did not\n\
         reproduce on this tiny dataset — example_kb's full table shape (wider\n\
         data struct, ~60 chunks, FTS index over real prose) is needed to trigger\n\
         it reliably. If the script aborted with a `Option::unwrap()` panic in\n\
         lance-encoding, you reproduced TODO-0159."
    );
    Ok(())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").to_string()
}
