#!/usr/bin/env rust-script
//! Root-cause spike for TODO-0159: filtering on a List<Float64> column panics
//! in lance-encoding 6.0. Isolate whether it's (a) Float64 lists, (b) lists in
//! general, (c) the element type, or (d) nesting inside a struct.
//!
//! Builds a table with several list columns of different element types and
//! runs an `only_if` filter that touches each, reporting OK / PANIC / error.
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

use arrow_array::builder::{Float64Builder, ListBuilder, StringBuilder, StructBuilder};
use arrow_array::{Int32Array, RecordBatch, RecordBatchIterator, RecordBatchReader, StructArray};
use arrow_schema::{DataType, Field, Fields, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};

fn list_type(item: DataType) -> DataType {
    DataType::List(Arc::new(Field::new("item", item, true)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("t.lance");
    let uri = uri.to_str().unwrap();

    // Mirror mdvs: a `data` Struct with a List<Float64> child (`f64s`) and a
    // List<Utf8> child (`strs`), where most rows have NULL lists (like
    // example_kb's measurement_values, present in only a couple of files).
    let f64s_field = Field::new("f64s", list_type(DataType::Float64), true);
    let strs_field = Field::new("strs", list_type(DataType::Utf8), true);
    let data_fields = Fields::from(vec![f64s_field.clone(), strs_field.clone()]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("data", DataType::Struct(data_fields.clone()), true),
    ]));

    // Match example_kb: 60 rows, only rows 0,1 have lists, of DIFFERENT
    // lengths (3 and 2), the other 58 NULL.
    const N: usize = 60;
    let id = Int32Array::from((0..N as i32).collect::<Vec<_>>());

    let mut sb = StructBuilder::new(
        data_fields.clone(),
        vec![
            Box::new(ListBuilder::new(Float64Builder::new())),
            Box::new(ListBuilder::new(StringBuilder::new())),
        ],
    );
    for i in 0..N {
        let f64l = sb.field_builder::<ListBuilder<Float64Builder>>(0).unwrap();
        if i == 0 {
            for v in [0.847, 0.853, 0.851] {
                f64l.values().append_value(v);
            }
            f64l.append(true);
        } else if i == 1 {
            for v in [0.612, 0.598] {
                f64l.values().append_value(v);
            }
            f64l.append(true);
        } else {
            f64l.append(false); // NULL list
        }
        let strl = sb.field_builder::<ListBuilder<StringBuilder>>(1).unwrap();
        if i < 2 {
            strl.values().append_value("a");
            strl.append(true);
        } else {
            strl.append(false);
        }
        sb.append(true); // data struct itself non-null
    }
    let data: StructArray = sb.finish();

    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(id), Arc::new(data)])?;

    let conn = lancedb::connect(uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
    let table = conn.create_table("t", reader).execute().await?;

    for (label, clause) in [
        ("data.f64s IS NOT NULL", "data.f64s IS NOT NULL"),
        ("data.strs IS NOT NULL", "data.strs IS NOT NULL"),
        ("array_has(data.f64s, 0.5)", "array_has(data.f64s, 0.5)"),
        ("array_has(data.strs, 'a')", "array_has(data.strs, 'a')"),
    ] {
        print!("{label:28} -> ");
        let res = std::panic::AssertUnwindSafe(async {
            table
                .query()
                .select(lancedb::query::Select::columns(&["id"]))
                .only_if(clause)
                .execute()
                .await?
                .try_collect::<Vec<RecordBatch>>()
                .await
        });
        match res.0.await {
            Ok(batches) => println!("OK ({} rows)", batches.iter().map(|b| b.num_rows()).sum::<usize>()),
            Err(e) => println!("ERR: {}", e.to_string().lines().next().unwrap_or("")),
        }
    }
    Ok(())
}
