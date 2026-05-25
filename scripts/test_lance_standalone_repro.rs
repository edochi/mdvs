#!/usr/bin/env rust-script
//! Fully self-contained reproducer for the lance-encoding 6.0 panic.
//!
//! No mdvs, no external file path. 2 KB Arrow IPC record-batch embedded
//! via `include_bytes!` from the file sitting next to this script. Single
//! batch, 79 rows, schema `id Int32 + data Struct{measurement_values List<Float64>}`,
//! 2 non-null `measurement_values` (lengths 3 and 2).
//!
//! Reading the IPC, handing the batch to `lancedb::create_table` with
//! storage version 2.1, then scanning `data.measurement_values` panics
//! deterministically at:
//!   lance-encoding-6.0.0/src/encodings/logical/primitive.rs:2505
//!   thread 'main' panicked: called `Option::unwrap()` on a `None` value
//!
//! Cross-check: constructing the same logical batch via Arrow's normal
//! builders (ListBuilder<Float64Builder>) and handing it to the same
//! create_table does NOT panic. The trigger is some property of the
//! IPC-reader-produced Arrow Buffers that survives schema/projection
//! manipulation but not a buffer deep copy.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! lance-index = "=6.0.0"
//! arrow = "58"
//! arrow-array = "58"
//! arrow-ipc = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! futures = "0.3"
//! tempfile = "3"
//! ```

use std::io::Cursor;

use arrow::ipc::reader::FileReader;
use arrow_array::{RecordBatchIterator, RecordBatchReader};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase, Select};

const IPC: &[u8] = include_bytes!("test_lance_mv_only.arrow");

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Embedded IPC blob: {} bytes", IPC.len());

    let mut rdr = FileReader::try_new(Cursor::new(IPC), None)?;
    let schema = rdr.schema();
    let batch = rdr.next().unwrap()?;
    println!(
        "Decoded batch: {} rows, {} top cols",
        batch.num_rows(),
        batch.num_columns()
    );
    for (i, f) in schema.fields().iter().enumerate() {
        println!("  col[{i}] {} {:?}", f.name(), f.data_type());
    }

    let dir = tempfile::tempdir()?;
    let uri = dir.path().to_string_lossy().to_string();
    println!("\nWriting via lancedb::create_table (storage v2.1): {uri}");
    let conn = lancedb::connect(&uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema));
    let table = conn
        .create_table("t", reader)
        .storage_option("new_table_data_storage_version", "2.1")
        .execute()
        .await?;
    println!("Wrote table.");

    println!("\nScanning data.measurement_values …");
    let s = table
        .query()
        .select(Select::columns(&["data.measurement_values"]))
        .execute()
        .await?;
    let batches: Vec<_> = s.try_collect().await?;
    let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    println!("Scan completed: {rows} rows. (You won't see this on the buggy version.)");
    Ok(())
}
