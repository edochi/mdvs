#!/usr/bin/env rust-script
//! Spike for TODO-0016 (script #5 / build metadata).
//!
//! mdvs stores seven `mdvs.*` keys (model, revision, chunk_size, glob,
//! built_at, schema_hash, dim) in parquet native key-value metadata today,
//! and hard-errors on model/revision/schema_hash mismatch. This proves the
//! Lance equivalent: `Table::replace_schema_metadata` to write,
//! `Table::schema().metadata()` to read.
//!
//! Also probes replace-vs-merge semantics (does writing one key drop the
//! others?) — relevant because incremental `build` re-touches metadata.
//!
//! PASS: all seven keys round-trip after write -> reopen.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! arrow-array = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator, RecordBatchReader};
use arrow_schema::{DataType, Field, Schema};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("index.lance");
    let uri = uri.to_str().unwrap();

    // the seven mdvs.* build-metadata keys
    let kv: Vec<(String, String)> = vec![
        ("mdvs.model", "minishlab/potion-base-8M"),
        ("mdvs.revision", "main"),
        ("mdvs.chunk_size", "512"),
        ("mdvs.glob", "**/*.md"),
        ("mdvs.built_at", "2026-05-22T10:00:00Z"),
        ("mdvs.schema_hash", "a1b2c3d4e5f6"),
        ("mdvs.dim", "256"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();

    // PATH A (preferred): bake metadata into the Arrow schema at create time.
    // All seven keys are known at build time, so this is the natural fit and
    // avoids the deprecation-flagged as_native() update path.
    let meta: HashMap<String, String> = kv.iter().cloned().collect();
    let schema = Arc::new(Schema::new_with_metadata(
        vec![Field::new("id", DataType::Int32, false)],
        meta,
    ));
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(vec![0, 1]))])?;

    let conn = lancedb::connect(uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
    let _table = conn.create_table("index", reader).execute().await?;

    // reopen + read
    drop(_table);
    drop(conn);
    let conn = lancedb::connect(uri).execute().await?;
    let table = conn.open_table("index").execute().await?;
    let md = table.schema().await?.metadata().clone();

    println!("PATH A (baked into schema at create) read-back after reopen:");
    let mut failures = 0;
    for (k, v) in &kv {
        match md.get(k) {
            Some(got) if got == v => println!("  {k:<20} = {got}"),
            Some(got) => {
                failures += 1;
                println!("  {k:<20} = {got}   MISMATCH (wanted {v})");
            }
            None => {
                failures += 1;
                println!("  {k:<20} = <MISSING>");
            }
        }
    }

    // PATH B: update one key via the (deprecation-flagged) NativeTable API,
    // and probe replace-vs-merge semantics.
    let native = table.as_native().expect("native table");
    native
        .replace_schema_metadata(vec![("mdvs.schema_hash".to_string(), "ffffff".to_string())])
        .await?;
    let md2 = table.schema().await?.metadata().clone();
    let merge = md2.contains_key("mdvs.model");
    println!(
        "\nafter rewriting only mdvs.schema_hash: other keys {} (semantics = {})",
        if merge { "survive" } else { "DROPPED" },
        if merge { "MERGE/upsert" } else { "REPLACE-all" }
    );
    println!("  -> wave 1: {}", if merge {
        "single-key updates are safe"
    } else {
        "build must re-write ALL seven keys every time"
    });

    println!();
    if failures == 0 {
        println!("PASS: all seven mdvs.* keys round-trip through Lance schema metadata.");
    } else {
        println!("{failures} key(s) failed to round-trip.");
    }
    Ok(())
}
