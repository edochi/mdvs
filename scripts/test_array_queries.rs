#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! datafusion = "52"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! ```

//! Test what DataFusion 52 supports for querying array (List) and nested
//! struct fields inside a parent Struct column, both directly and through
//! a view that promotes Struct children to top-level columns.

use datafusion::arrow::array::{
    Array, ArrayRef, BooleanArray, ListArray, StringArray, StructArray,
};
use datafusion::arrow::buffer::OffsetBuffer;
use datafusion::arrow::datatypes::{DataType, Field, Fields, Schema};
use datafusion::arrow::record_batch::RecordBatch;
use datafusion::execution::context::SessionContext;
use datafusion::parquet::arrow::ArrowWriter;
use std::sync::Arc;
use tempfile::tempdir;

// ============================================================================
// Test data — 3 files with array and nested struct fields
// ============================================================================
//
// file  | tags              | meta.source | meta.reviewed | draft
// ------|-------------------|-------------|---------------|------
// f1    | [rust, traits]    | web         | true          | false
// f2    | [python, async]   | book        | false         | false
// f3    | [rust, async]     | web         | true          | true
//

fn write_test_parquet(path: &std::path::Path) {
    // Inner struct: meta { source: String, reviewed: Boolean }
    let meta_fields = Fields::from(vec![
        Field::new("source", DataType::Utf8, true),
        Field::new("reviewed", DataType::Boolean, true),
    ]);

    let meta_sources = StringArray::from(vec![Some("web"), Some("book"), Some("web")]);
    let meta_reviewed = BooleanArray::from(vec![Some(true), Some(false), Some(true)]);
    let meta_arr = StructArray::new(
        meta_fields.clone(),
        vec![
            Arc::new(meta_sources) as ArrayRef,
            Arc::new(meta_reviewed) as ArrayRef,
        ],
        None,
    );

    // Array field: tags List<Utf8>
    let tag_values = StringArray::from(vec!["rust", "traits", "python", "async", "rust", "async"]);
    let tag_offsets = OffsetBuffer::new(vec![0i32, 2, 4, 6].into());
    let tags_arr = ListArray::new(
        Arc::new(Field::new("item", DataType::Utf8, true)),
        tag_offsets,
        Arc::new(tag_values),
        None,
    );

    // Scalar field: draft Boolean
    let drafts = BooleanArray::from(vec![Some(false), Some(false), Some(true)]);

    // Outer data Struct with tags, meta, draft
    let data_fields = Fields::from(vec![
        Field::new(
            "tags",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            true,
        ),
        Field::new("meta", DataType::Struct(meta_fields), true),
        Field::new("draft", DataType::Boolean, true),
    ]);

    let data_arr = StructArray::new(
        data_fields.clone(),
        vec![
            Arc::new(tags_arr) as ArrayRef,
            Arc::new(meta_arr) as ArrayRef,
            Arc::new(drafts) as ArrayRef,
        ],
        None,
    );

    let schema = Schema::new(vec![
        Field::new("file_id", DataType::Utf8, false),
        Field::new("filename", DataType::Utf8, false),
        Field::new("data", DataType::Struct(data_fields), true),
    ]);

    let batch = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(StringArray::from(vec!["f1", "f2", "f3"])),
            Arc::new(StringArray::from(vec!["a.md", "b.md", "c.md"])),
            Arc::new(data_arr),
        ],
    )
    .unwrap();

    let file = std::fs::File::create(path).unwrap();
    let mut writer = ArrowWriter::try_new(file, batch.schema(), None).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

/// Create the same view mdvs uses: promote _data Struct children to top-level.
/// Only goes one level deep (same as production code).
async fn create_view(ctx: &SessionContext) {
    let files_table = ctx.table("files").await.unwrap();
    let schema = files_table.schema();
    let mut projections = Vec::new();
    for field in schema.fields() {
        if field.name() == "data" {
            if let DataType::Struct(children) = field.data_type() {
                for child in children {
                    projections.push(format!(
                        "data['{}'] AS \"{}\"",
                        child.name(),
                        child.name()
                    ));
                }
            }
        }
    }
    let extra = if projections.is_empty() {
        String::new()
    } else {
        format!(", {}", projections.join(", "))
    };
    let sql = format!("CREATE VIEW files_v AS SELECT *{extra} FROM files");
    println!("  VIEW SQL: {sql}");
    ctx.sql(&sql).await.unwrap();
}

fn try_query(label: &str, result: Result<Vec<RecordBatch>, impl std::fmt::Debug>) {
    match result {
        Ok(batches) => {
            let rows: usize = batches.iter().map(|b| b.num_rows()).sum();
            println!("  {label}: OK ({rows} rows)");
            for batch in &batches {
                for col_idx in 0..batch.num_columns() {
                    let col = batch.column(col_idx);
                    let schema = batch.schema();
                    let name = schema.field(col_idx).name();
                    println!("    {name} ({:?}): {col:?}", col.data_type());
                }
            }
        }
        Err(e) => {
            println!("  {label}: FAILED — {e:?}");
        }
    }
}

async fn run_query(ctx: &SessionContext, sql: &str) -> Result<Vec<RecordBatch>, String> {
    let df = ctx.sql(sql).await.map_err(|e| format!("parse: {e}"))?;
    df.collect().await.map_err(|e| format!("exec: {e}"))
}

#[tokio::main]
async fn main() {
    println!("=== Array & Struct query tests (DataFusion 52) ===\n");

    let tmp = tempdir().unwrap();
    let path = tmp.path().join("files.parquet");
    write_test_parquet(&path);

    let ctx = SessionContext::new();
    ctx.register_parquet("files", path.to_str().unwrap(), Default::default())
        .await
        .unwrap();
    create_view(&ctx).await;

    // ========================================================================
    // Part 1: Array queries
    // ========================================================================
    println!("\n--- Part 1: Array queries ---\n");

    // 1a. array_has on raw Struct accessor
    println!("1a. array_has via bracket accessor:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files WHERE array_has(data['tags'], 'rust')",
    )
    .await;
    try_query("array_has(data['tags'], 'rust')", r);

    // 1b. array_has on promoted view column
    println!("\n1b. array_has via view column:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_has(tags, 'rust')",
    )
    .await;
    try_query("array_has(tags, 'rust')", r);

    // 1c. array_contains (alias?)
    println!("\n1c. array_contains via view column:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_contains(tags, 'rust')",
    )
    .await;
    try_query("array_contains(tags, 'rust')", r);

    // 1d. = ANY() syntax
    println!("\n1d. = ANY() syntax:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE 'rust' = ANY(tags)",
    )
    .await;
    try_query("'rust' = ANY(tags)", r);

    // 1e. array_length
    println!("\n1e. array_length via view:");
    let r = run_query(
        &ctx,
        "SELECT filename, array_length(tags) AS len FROM files_v",
    )
    .await;
    try_query("array_length(tags)", r);

    // 1f. array_has with multiple conditions
    println!("\n1f. array_has with AND:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_has(tags, 'rust') AND array_has(tags, 'async')",
    )
    .await;
    try_query("array_has(tags, 'rust') AND array_has(tags, 'async')", r);

    // 1g. array_has combined with scalar filter
    println!("\n1g. array_has + scalar filter:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_has(tags, 'rust') AND draft = false",
    )
    .await;
    try_query("array_has(tags, 'rust') AND draft = false", r);

    // ========================================================================
    // Part 2: Nested Struct queries
    // ========================================================================
    println!("\n--- Part 2: Nested Struct queries ---\n");

    // 2a. meta is promoted as a Struct — what type does the view give it?
    println!("2a. SELECT meta from view (check type):");
    let r = run_query(&ctx, "SELECT meta FROM files_v").await;
    try_query("SELECT meta", r);

    // 2b. Nested access: meta['source'] via view
    println!("\n2b. meta['source'] via view:");
    let r = run_query(
        &ctx,
        "SELECT filename, meta['source'] AS src FROM files_v",
    )
    .await;
    try_query("meta['source']", r);

    // 2c. Nested access: data['meta']['source'] via raw table
    println!("\n2c. data['meta']['source'] via raw table:");
    let r = run_query(
        &ctx,
        "SELECT filename, data['meta']['source'] AS src FROM files",
    )
    .await;
    try_query("data['meta']['source']", r);

    // 2d. Filter on nested struct field via view
    println!("\n2d. WHERE meta['source'] = 'web' via view:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE meta['source'] = 'web'",
    )
    .await;
    try_query("meta['source'] = 'web'", r);

    // 2e. Filter on nested struct field via raw table
    println!("\n2e. WHERE data['meta']['source'] = 'web' via raw table:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files WHERE data['meta']['source'] = 'web'",
    )
    .await;
    try_query("data['meta']['source'] = 'web'", r);

    // 2f. Combine nested struct + array query
    println!("\n2f. array_has + nested struct filter:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_has(tags, 'rust') AND meta['source'] = 'web'",
    )
    .await;
    try_query("array_has(tags, 'rust') AND meta['source'] = 'web'", r);

    // 2g. Boolean nested field
    println!("\n2g. meta['reviewed'] = true via view:");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE meta['reviewed'] = true",
    )
    .await;
    try_query("meta['reviewed'] = true", r);

    // ========================================================================
    // Part 3: Edge cases
    // ========================================================================
    println!("\n--- Part 3: Edge cases ---\n");

    // 3a. Empty array handling (not in test data, but good to know)
    println!("3a. array_has on NULL tags (if any):");
    let r = run_query(
        &ctx,
        "SELECT filename FROM files_v WHERE array_has(tags, 'nonexistent')",
    )
    .await;
    try_query("array_has(tags, 'nonexistent')", r);

    // 3b. Unnest
    println!("\n3b. UNNEST(tags) via view:");
    let r = run_query(
        &ctx,
        "SELECT filename, UNNEST(tags) AS tag FROM files_v",
    )
    .await;
    try_query("UNNEST(tags)", r);

    // 3c. Unnest with GROUP BY (find files sharing a tag)
    println!("\n3c. UNNEST + GROUP BY:");
    let r = run_query(
        &ctx,
        "SELECT UNNEST(tags) AS tag, COUNT(*) AS cnt FROM files_v GROUP BY tag ORDER BY cnt DESC",
    )
    .await;
    try_query("UNNEST + GROUP BY", r);

    println!("\n=== Done ===");
}
