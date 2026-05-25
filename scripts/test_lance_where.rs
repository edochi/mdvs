#!/usr/bin/env rust-script
//! Spike for TODO-0016 (script #3 / filter parity — the real risk).
//!
//! Runs a battery of `--where`-style filter clauses against a denormalized
//! table whose frontmatter lives in a nested `data` Struct, to learn:
//!   1. Which nested-access syntax LanceDB's filter parser accepts
//!      (plain `data.calibration.baseline.wavelength` vs backticked).
//!   2. That every clause family in book/src/search-guide.md survives the
//!      backend swap: nested struct, Date/Timestamp literals, array_has,
//!      IN, BETWEEN, LIKE, IS NULL, AND/OR.
//!
//! Each clause prints expected vs actual row count (or the parser error),
//! so we map exactly what the dot->backtick translator must emit.
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

use arrow_array::builder::{ListBuilder, StringBuilder};
use arrow_array::{
    Array, Date32Array, Int32Array, RecordBatch, RecordBatchIterator, RecordBatchReader,
    StringArray, StructArray, TimestampMicrosecondArray,
};
use arrow_schema::{DataType, Field, Fields, Schema, TimeUnit};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::Table;

// days since epoch
const D_2023_01_15: i32 = 19372;
const D_2024_06_01: i32 = 19875;
const D_2024_12_31: i32 = 20088;
// micros since epoch
const T_2023_01_15: i64 = 1_673_771_400_000_000; // 08:30:00Z
const T_2024_06_01: i64 = 1_717_243_200_000_000; // 12:00:00Z
const T_2024_12_31: i64 = 1_735_689_540_000_000; // 23:59:00Z

fn data_struct_type() -> DataType {
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
    let tags = Field::new(
        "tags",
        DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        true,
    );
    DataType::Struct(Fields::from(vec![
        Field::new("title", DataType::Utf8, true),
        Field::new("rating", DataType::Int32, true),
        Field::new("published", DataType::Date32, true),
        Field::new(
            "synced_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            true,
        ),
        Field::new("maybe_null", DataType::Utf8, true),
        tags,
        calibration,
    ]))
}

fn build_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("chunk_id", DataType::Utf8, false),
        Field::new("file_id", DataType::Utf8, false),
        Field::new("data", data_struct_type(), true),
    ]))
}

fn build_data_array() -> StructArray {
    let titles = StringArray::from(vec!["Dune", "Foundation", "Spirited Away"]);
    let ratings = Int32Array::from(vec![5, 4, 5]);
    let published = Date32Array::from(vec![D_2024_06_01, D_2023_01_15, D_2024_12_31]);
    let synced = TimestampMicrosecondArray::from(vec![T_2024_06_01, T_2023_01_15, T_2024_12_31])
        .with_timezone("UTC");
    let maybe_null = StringArray::from(vec![Some("x"), None, Some("y")]);

    let mut tb = ListBuilder::new(StringBuilder::new());
    tb.values().append_value("sci-fi");
    tb.values().append_value("classic");
    tb.append(true);
    tb.values().append_value("sci-fi");
    tb.append(true);
    tb.values().append_value("fantasy");
    tb.append(true);
    let tags = tb.finish();

    let wavelengths = arrow_array::Float64Array::from(vec![850.0, 632.8, 905.0]);
    let baseline = StructArray::from(vec![(
        Arc::new(Field::new("wavelength", DataType::Float64, true)),
        Arc::new(wavelengths) as Arc<dyn Array>,
    )]);
    let calibration = StructArray::from(vec![(
        Arc::new(Field::new("baseline", baseline.data_type().clone(), true)),
        Arc::new(baseline) as Arc<dyn Array>,
    )]);

    StructArray::from(vec![
        (
            Arc::new(Field::new("title", DataType::Utf8, true)),
            Arc::new(titles) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new("rating", DataType::Int32, true)),
            Arc::new(ratings) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new("published", DataType::Date32, true)),
            Arc::new(published) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new(
                "synced_at",
                DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                true,
            )),
            Arc::new(synced) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new("maybe_null", DataType::Utf8, true)),
            Arc::new(maybe_null) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new(
                "tags",
                DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
                true,
            )),
            Arc::new(tags) as Arc<dyn Array>,
        ),
        (
            Arc::new(Field::new(
                "calibration",
                calibration.data_type().clone(),
                true,
            )),
            Arc::new(calibration) as Arc<dyn Array>,
        ),
    ])
}

async fn run_filter(table: &Table, clause: &str) -> Result<usize, String> {
    let stream = table
        .query()
        .only_if(clause)
        .execute()
        .await
        .map_err(|e| e.to_string())?;
    let batches: Vec<RecordBatch> = stream.try_collect().await.map_err(|e| e.to_string())?;
    Ok(batches.iter().map(|b| b.num_rows()).sum())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("index.lance");
    let uri = uri.to_str().unwrap();

    let schema = build_schema();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["c0", "c1", "c2"])),
            Arc::new(StringArray::from(vec!["f0", "f1", "f2"])),
            Arc::new(build_data_array()),
        ],
    )?;

    let conn = lancedb::connect(uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
    let table = conn.create_table("index", reader).execute().await?;

    // (label, clause, expected-rows-or-None-to-just-report)
    let cases: &[(&str, &str, Option<usize>)] = &[
        // --- nested struct access: which syntax does the parser accept? ---
        (
            "nested plain dotted",
            "data.calibration.baseline.wavelength > 800",
            Some(2),
        ),
        (
            "nested backticked",
            "`data`.`calibration`.`baseline`.`wavelength` > 800",
            Some(2),
        ),
        (
            "nested mixed (data plain, rest backtick)",
            "data.`calibration`.`baseline`.`wavelength` > 800",
            None,
        ),
        // --- one-level nested scalar ---
        ("scalar plain dotted", "data.rating IN (4, 5)", Some(3)),
        ("scalar backticked", "`data`.`rating` IN (4, 5)", Some(3)),
        // --- literal families ---
        ("Date literal", "data.published >= date '2024-01-01'", Some(2)),
        (
            "Timestamp literal",
            "data.synced_at >= timestamp '2024-01-01T00:00:00Z'",
            Some(2),
        ),
        ("BETWEEN", "data.rating BETWEEN 5 AND 5", Some(2)),
        ("LIKE", "data.title LIKE 'D%'", Some(1)),
        ("IS NULL", "data.maybe_null IS NULL", Some(1)),
        ("IS NOT NULL", "data.maybe_null IS NOT NULL", Some(2)),
        // --- array membership ---
        ("array_has", "array_has(data.tags, 'sci-fi')", Some(2)),
        // --- boolean composition ---
        (
            "AND across nested + scalar",
            "data.rating = 5 AND data.calibration.baseline.wavelength > 800",
            Some(2),
        ),
        (
            "OR",
            "data.title LIKE 'D%' OR data.rating = 4",
            Some(2),
        ),
    ];

    let mut failures = 0;
    println!("{:<42} {:>8}  {}", "clause", "rows", "result");
    println!("{}", "-".repeat(78));
    for (label, clause, expected) in cases {
        match run_filter(&table, clause).await {
            Ok(n) => {
                let verdict = match expected {
                    Some(e) if *e == n => "OK".to_string(),
                    Some(e) => {
                        failures += 1;
                        format!("MISMATCH (expected {e})")
                    }
                    None => "(report)".to_string(),
                };
                println!("{label:<42} {n:>8}  {verdict}");
            }
            Err(e) => {
                if expected.is_some() {
                    failures += 1;
                }
                let short = e.lines().next().unwrap_or("").chars().take(60).collect::<String>();
                println!("{label:<42} {:>8}  PARSE-ERR: {short}", "-");
            }
        }
    }

    println!("{}", "-".repeat(78));
    if failures == 0 {
        println!("PASS: all expected clauses returned the right row counts.");
    } else {
        println!("{failures} clause(s) failed — see above.");
    }
    Ok(())
}
