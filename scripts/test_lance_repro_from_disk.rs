#!/usr/bin/env rust-script
//! Open an existing `.mdvs/index.lance` directory directly with pure
//! lancedb (no mdvs runtime) and run the failing query. If this panics,
//! the offending bytes are on disk and the reproducer is "this lance dir
//! + this snippet" — independent of mdvs.
//!
//! Usage: `rust-script test_lance_repro_from_disk.rs path/to/.mdvs`
//! (defaults to `example_kb/.mdvs` if no arg).
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! lance-index = "=6.0.0"
//! arrow = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! futures = "0.3"
//! ```

use std::time::Duration;

use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::query::{ExecutableQuery, QueryBase, Select};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "example_kb/.mdvs".to_string());
    println!("Opening Lance database at: {arg}");
    let conn = lancedb::connect(&arg).execute().await?;
    let table = conn.open_table("index").execute().await?;
    println!("Opened table 'index', row count = {}", table.count_rows(None).await?);

    // Read the embedding dim from the schema so we can build a query vector.
    let schema = table.schema().await?;
    let emb_field = schema.field_with_name("embedding")?;
    let dim = match emb_field.data_type() {
        arrow::datatypes::DataType::FixedSizeList(_, d) => *d as usize,
        _ => return Err("embedding is not FixedSizeList".into()),
    };
    let query_vec = vec![1.0f32; dim];
    println!("Embedding dim = {dim}\n");

    // Each query is a candidate panic-trigger. We use a watchdog timer so a
    // worker-thread panic (which hangs the main task) is reported instead of
    // wedging the script.
    for label in [
        "plain only_if scan",
        "nearest_to + only_if (≈ semantic)",
        "nearest_to + full_text_search + only_if (≈ hybrid)",
        "full_text_search + only_if (≈ fulltext)",
    ] {
        let qv = query_vec.clone();
        let table = table.clone();
        let fut = async move {
            match label {
                "plain only_if scan" => {
                    let s = table
                        .query()
                        .select(Select::columns(&["file_id"]))
                        .only_if("data.measurement_values IS NOT NULL")
                        .execute()
                        .await?;
                    let batches: Vec<_> = s.try_collect().await?;
                    Ok::<usize, lancedb::Error>(batches.iter().map(|b| b.num_rows()).sum())
                }
                "nearest_to + only_if (≈ semantic)" => {
                    let s = table
                        .query()
                        .nearest_to(qv)?
                        .distance_type(lancedb::DistanceType::Cosine)
                        .select(Select::columns(&["file_id"]))
                        .only_if("data.measurement_values IS NOT NULL")
                        .limit(20)
                        .execute()
                        .await?;
                    let batches: Vec<_> = s.try_collect().await?;
                    Ok(batches.iter().map(|b| b.num_rows()).sum())
                }
                "nearest_to + full_text_search + only_if (≈ hybrid)" => {
                    let s = table
                        .query()
                        .nearest_to(qv)?
                        .distance_type(lancedb::DistanceType::Cosine)
                        .full_text_search(FullTextSearchQuery::new("calibration".to_string()))
                        .select(Select::columns(&["file_id"]))
                        .only_if("data.measurement_values IS NOT NULL")
                        .limit(20)
                        .execute()
                        .await?;
                    let batches: Vec<_> = s.try_collect().await?;
                    Ok(batches.iter().map(|b| b.num_rows()).sum())
                }
                _ => {
                    let s = table
                        .query()
                        .full_text_search(FullTextSearchQuery::new("calibration".to_string()))
                        .select(Select::columns(&["file_id"]))
                        .only_if("data.measurement_values IS NOT NULL")
                        .limit(20)
                        .execute()
                        .await?;
                    let batches: Vec<_> = s.try_collect().await?;
                    Ok(batches.iter().map(|b| b.num_rows()).sum())
                }
            }
        };
        print!("  {label:55} → ");
        match tokio::time::timeout(Duration::from_secs(15), fut).await {
            Ok(Ok(n)) => println!("OK (n={n})"),
            Ok(Err(e)) => println!("ERR: {}", e.to_string().lines().next().unwrap_or("")),
            Err(_) => println!("TIMEOUT (likely worker-thread panic → TODO-0159)"),
        }
    }
    Ok(())
}
