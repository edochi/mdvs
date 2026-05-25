#!/usr/bin/env rust-script
//! Inspect a Lance dataset directory at the manifest / fragment / file level
//! and print what the writer chose: format version, fragment count, file
//! paths, schema (incl. metadata + child types), and basic file-reader
//! statistics for the `data` Struct column.
//!
//! The point is to diff example_kb's index.lance against a synthetic spike
//! so we know which writer-side knob makes Lance pick the panicky decoder
//! for TODO-0159.
//!
//! Usage: `rust-script test_lance_inspect.rs path/to/index.lance`
//! (defaults to `example_kb/.mdvs/index.lance`).
//!
//! ```cargo
//! [dependencies]
//! lance = "=6.0.0"
//! arrow = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! futures = "0.3"
//! ```

use arrow_schema::DataType;
use futures::TryStreamExt;
use lance::Dataset;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let uri = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "example_kb/.mdvs/index.lance".to_string());
    println!("Opening Lance dataset: {uri}\n");

    let dataset = Dataset::open(&uri).await?;

    println!("=== Manifest ===");
    println!("  dataset version       : {}", dataset.version().version);
    println!("  manifest read version : {}", dataset.manifest().version);
    let storage = dataset.manifest().data_storage_format.clone();
    println!(
        "  data storage format   : {} (version {})",
        storage.file_format, storage.version
    );
    println!("  fragment count        : {}", dataset.get_fragments().len());
    println!(
        "  total rows            : {}",
        dataset.count_rows(None).await?
    );

    println!("\n=== Top-level schema ===");
    let schema = dataset.schema();
    for f in schema.fields.iter() {
        println!(
            "  - {:24} {:?}  (id={}, nullable={})",
            f.name, f.data_type(), f.id, f.nullable
        );
    }

    // Drill into `data` Struct children for the panic-relevant column.
    if let Some(data_field) = schema.fields.iter().find(|f| f.name == "data") {
        println!("\n=== `data` Struct children ({} fields) ===", data_field.children.len());
        for child in &data_field.children {
            println!(
                "  - {:36} {:?}  (id={}, nullable={})",
                child.name, child.data_type(), child.id, child.nullable
            );
            for grandchild in &child.children {
                println!(
                    "      - {:32} {:?}  (id={})",
                    grandchild.name, grandchild.data_type(), grandchild.id
                );
            }
        }
    }

    println!("\n=== Fragments ===");
    for (i, frag) in dataset.get_fragments().iter().enumerate() {
        let meta = frag.metadata();
        let rows = frag.count_rows(None).await?;
        println!(
            "  fragment[{i}] id={} rows={}",
            meta.id, rows
        );
        for file in &meta.files {
            println!(
                "    file: {}  (major={} minor={}, fields={:?})",
                file.path, file.file_major_version, file.file_minor_version, file.fields
            );
        }
    }

    // Try opening one fragment's data file to inspect the lance file reader's
    // view of it (page count, etc.).
    if let Some(frag) = dataset.get_fragments().first() {
        println!("\n=== Reading first fragment to count batches/rows per scan ===");
        let mut stream = frag.scan().project(&["file_id"])?.try_into_stream().await?;
        let mut batches = 0usize;
        let mut rows = 0usize;
        while let Some(b) = stream.try_next().await? {
            batches += 1;
            rows += b.num_rows();
        }
        println!("  file_id scan: {batches} batches, {rows} rows");

        // Now try projecting the panic column. This may panic; we wrap in
        // a tokio timeout so the script reports it instead of hanging.
        let q_col = "data";
        println!(
            "\n=== Scanning '{q_col}' on first fragment (may panic on TODO-0159) ==="
        );
        let mut scanner = frag.scan();
        let project_res = scanner.project(&[q_col]);
        let scanner = match project_res {
            Ok(s) => s,
            Err(e) => {
                println!("  project failed: {e}");
                return Ok(());
            }
        };
        let stream_res = scanner.try_into_stream().await;
        let mut stream = match stream_res {
            Ok(s) => s,
            Err(e) => {
                println!("  open stream failed: {e}");
                return Ok(());
            }
        };
        let fut = async {
            let mut b = 0usize;
            let mut r = 0usize;
            while let Some(batch) = stream.try_next().await? {
                b += 1;
                r += batch.num_rows();
                // Report the per-batch schema of `data` so we see how Lance
                // materialised the Struct.
                if b == 1 {
                    if let Some(col) = batch.column_by_name("data") {
                        println!("    first batch `data` DataType: {:?}", col.data_type());
                        if let DataType::Struct(fields) = col.data_type() {
                            for f in fields.iter() {
                                if matches!(
                                    f.data_type(),
                                    DataType::List(_) | DataType::LargeList(_)
                                ) {
                                    println!(
                                        "      → list child: {} = {:?}",
                                        f.name(),
                                        f.data_type()
                                    );
                                }
                            }
                        }
                    }
                }
            }
            Ok::<_, lance::Error>((b, r))
        };
        match tokio::time::timeout(std::time::Duration::from_secs(15), fut).await {
            Ok(Ok((b, r))) => println!("  data scan: {b} batches, {r} rows"),
            Ok(Err(e)) => println!("  data scan errored: {e}"),
            Err(_) => println!("  data scan TIMEOUT (likely the panic)"),
        }
    }

    Ok(())
}
