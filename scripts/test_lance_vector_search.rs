#!/usr/bin/env rust-script
//! Spike for TODO-0016 (script #2 / semantic search, BYO embedding).
//!
//! Inserts N rows of deterministic pseudo-random Float32 vectors, then asks
//! LanceDB for the top-K nearest to a query vector using COSINE distance
//! (mdvs's metric — LanceDB defaults to L2, so we set it explicitly).
//! Compares the returned ranking against a brute-force cosine ranking
//! computed in-script.
//!
//! PASS: top-K id ordering from LanceDB matches brute-force cosine exactly
//! (no ANN index built, so LanceDB does exact kNN).
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

use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int32Array, RecordBatch, RecordBatchIterator,
    RecordBatchReader,
};
use arrow_schema::{DataType, Field, Schema};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::DistanceType;

const DIM: usize = 16;
const N: usize = 50;
const K: usize = 5;

// deterministic LCG -> f32 in [-1, 1]
struct Lcg(u64);
impl Lcg {
    fn next_f32(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let bits = (self.0 >> 33) as u32;
        (bits as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let uri = dir.path().join("index.lance");
    let uri = uri.to_str().unwrap();

    // generate vectors
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let mut vecs: Vec<Vec<f32>> = Vec::with_capacity(N);
    for _ in 0..N {
        vecs.push((0..DIM).map(|_| rng.next_f32()).collect());
    }
    let query: Vec<f32> = (0..DIM).map(|_| rng.next_f32()).collect();

    // schema: id + embedding
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new(
            "embedding",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                DIM as i32,
            ),
            true,
        ),
    ]));

    let ids = Int32Array::from((0..N as i32).collect::<Vec<_>>());
    let flat: Vec<f32> = vecs.iter().flatten().copied().collect();
    let embedding = FixedSizeListArray::new(
        Arc::new(Field::new("item", DataType::Float32, true)),
        DIM as i32,
        Arc::new(Float32Array::from(flat)),
        None,
    );
    let batch = RecordBatch::try_new(schema.clone(), vec![Arc::new(ids), Arc::new(embedding)])?;

    let conn = lancedb::connect(uri).execute().await?;
    let reader: Box<dyn RecordBatchReader + Send> =
        Box::new(RecordBatchIterator::new(vec![Ok(batch)], schema.clone()));
    let table = conn.create_table("index", reader).execute().await?;

    // --- brute-force cosine ranking ---
    let mut scored: Vec<(i32, f32)> = vecs
        .iter()
        .enumerate()
        .map(|(i, v)| (i as i32, cosine(&query, v)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    let brute_topk: Vec<i32> = scored.iter().take(K).map(|(id, _)| *id).collect();

    // --- LanceDB cosine kNN ---
    let stream = table
        .query()
        .nearest_to(query.clone())?
        .distance_type(DistanceType::Cosine)
        .limit(K)
        .execute()
        .await?;
    let batches: Vec<RecordBatch> = stream.try_collect().await?;
    let mut lance_topk: Vec<i32> = Vec::new();
    let mut lance_dist: Vec<f32> = Vec::new();
    for b in &batches {
        let id = b
            .column_by_name("id")
            .unwrap()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        let dist = b
            .column_by_name("_distance")
            .unwrap()
            .as_any()
            .downcast_ref::<Float32Array>()
            .unwrap();
        for i in 0..b.num_rows() {
            lance_topk.push(id.value(i));
            lance_dist.push(dist.value(i));
        }
    }

    println!("query dim = {DIM}, corpus = {N}, K = {K}\n");
    println!("{:<6} {:<22} {:<22}", "rank", "brute-force (id, sim)", "lancedb (id, 1-dist)");
    for r in 0..K {
        let (bid, bsim) = scored[r];
        let lid = lance_topk[r];
        let lsim = 1.0 - lance_dist[r]; // cosine distance -> similarity
        let mark = if bid == lid { "" } else { "  <-- DIFF" };
        println!(
            "{:<6} {:<22} {:<22}{}",
            r,
            format!("{bid}, {bsim:.4}"),
            format!("{lid}, {lsim:.4}"),
            mark
        );
    }

    println!();
    if brute_topk == lance_topk {
        println!("PASS: LanceDB cosine top-{K} matches brute-force exactly.");
    } else {
        println!("FAIL: ranking differs.");
        println!("  brute: {brute_topk:?}");
        println!("  lance: {lance_topk:?}");
    }
    Ok(())
}
