#!/usr/bin/env rust-script
//! Spike for TODO-0016 (script #4 / fulltext + hybrid — NEW capabilities).
//!
//! These aren't parity checks (mdvs has no FTS today) — they prove the two
//! new search modes the swap unlocks:
//!   1. BM25 fulltext over a persisted `chunk_text` column, and that the
//!      default tokenizer (whitespace + lowercase) is sane for English md.
//!   2. Hybrid (vector + FTS, fused by LanceDB's default RRF reranker):
//!      a doc matching BOTH a keyword and being vector-near ranks first.
//!
//! Corpus (id : chunk_text), hand-set 4-d embeddings:
//!   0 "the quick brown fox jumps over the lazy dog"   [1,0,0,0]
//!   1 "machine learning models require training data"  [0,1,0,0]
//!   2 "the dog barked loudly at the mailman"           [0,0,1,0]
//!   3 "neural networks are a class of machine learning"[0,0,0,1]
//!   4 "a fox is a small wild canine"                   [0.9,0.1,0,0]
//!   5 "deep learning uses neural networks"             [0,0,0.1,0.9]
//!
//! PASS: FTS "dog" returns exactly {0,2}; hybrid("dog", vec≈doc2) ranks
//! doc2 first (matches keyword AND vector-nearest).
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.29"
//! lance-index = "=6.0.0"
//! arrow-array = "58"
//! arrow-schema = "58"
//! tokio = { version = "1", features = ["full"] }
//! tempfile = "3"
//! futures = "0.3"
//! ```

use std::sync::Arc;

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::index::Index;
use lancedb::query::{ExecutableQuery, QueryBase};

const DIM: i32 = 4;

fn ids_from(batches: &[RecordBatch]) -> Vec<i32> {
    let mut out = Vec::new();
    for b in batches {
        let id = b
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        for i in 0..b.num_rows() {
            out.push(id.value(i));
        }
    }
    out
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("index.lance");
    let uri = uri.to_str().unwrap();

    let texts = vec![
        "the quick brown fox jumps over the lazy dog",
        "machine learning models require training data",
        "the dog barked loudly at the mailman",
        "neural networks are a class of machine learning",
        "a fox is a small wild canine",
        "deep learning uses neural networks",
    ];
    let embs: Vec<[f32; 4]> = vec![
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
        [0.9, 0.1, 0.0, 0.0],
        [0.0, 0.0, 0.1, 0.9],
    ];

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("chunk_text", DataType::Utf8, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
            true,
        ),
    ]));

    let ids = Int32Array::from((0..texts.len() as i32).collect::<Vec<_>>());
    let text_arr = StringArray::from(texts.clone());
    let flat: Vec<f32> = embs.iter().flatten().copied().collect();
    let embedding = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        DIM,
        Arc::new(Float32Array::from(flat)),
        None,
    );
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(ids), Arc::new(text_arr), Arc::new(embedding)],
    )?;

    let conn = lancedb::connect(uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
    let table = conn.create_table("index", reader).execute().await?;

    // build the BM25 / inverted FTS index on chunk_text
    table
        .create_index(&["chunk_text"], Index::FTS(Default::default()))
        .execute()
        .await?;
    println!("FTS index built on chunk_text\n");

    let mut failures = 0;

    // --- 1. fulltext "dog" ---
    let fts_batches: Vec<RecordBatch> = table
        .query()
        .full_text_search(FullTextSearchQuery::new("dog".to_string()))
        .limit(10)
        .execute()
        .await?
        .try_collect()
        .await?;
    let mut fts_ids = ids_from(&fts_batches);
    fts_ids.sort();
    println!("FTS 'dog' -> ids {fts_ids:?}  (expected [0, 2])");
    if !fts_batches.is_empty() {
        let cols: Vec<&str> = fts_batches[0]
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().as_str())
            .map(|s| Box::leak(s.to_string().into_boxed_str()) as &str)
            .collect();
        println!("  result columns: {cols:?}");
    }
    if fts_ids != vec![0, 2] {
        failures += 1;
        println!("  MISMATCH");
    }

    // --- 2. hybrid: text "dog" + vector near doc2 ---
    let hybrid_batches: Vec<RecordBatch> = table
        .query()
        .full_text_search(FullTextSearchQuery::new("dog".to_string()))
        .nearest_to(&[0.0, 0.0, 1.0, 0.0])?
        .limit(5)
        .execute()
        .await?
        .try_collect()
        .await?;
    let hybrid_ids = ids_from(&hybrid_batches);
    println!("\nhybrid('dog', vec≈doc2) -> ranked ids {hybrid_ids:?}");
    if !hybrid_batches.is_empty() {
        let cols: Vec<String> = hybrid_batches[0]
            .schema()
            .fields()
            .iter()
            .map(|f| f.name().clone())
            .collect();
        println!("  result columns: {cols:?}");
    }
    match hybrid_ids.first() {
        Some(2) => println!("  doc2 ranked first (matches keyword AND vector)"),
        other => {
            failures += 1;
            println!("  MISMATCH: expected doc2 first, got {other:?}");
        }
    }

    println!();
    if failures == 0 {
        println!("PASS: BM25 fulltext + RRF hybrid both behave as expected.");
    } else {
        println!("{failures} check(s) failed.");
    }
    Ok(())
}
