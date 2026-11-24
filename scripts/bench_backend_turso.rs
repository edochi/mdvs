#!/usr/bin/env rust-script
//! Backend benchmark: Turso (SQLite-compatible, in Rust).
//!
//! Usage: `bench_backend_turso.rs <dataset_dir> <work_dir> [reps]`
//!
//! Same workload and output as `bench_backend_lance.rs`. Turso has no
//! approximate vector index, so semantic search is an exact scan
//! (`vector_distance_cos`). Full-text search uses Turso's Tantivy-backed FTS
//! index (experimental, enabled with `experimental_index_method`). Hybrid
//! fuses the vector and BM25 top-k with reciprocal rank fusion (k = 60), as
//! LanceDB's default reranker does. Frontmatter is a JSON TEXT column,
//! filtered with `json_extract`.
//!
//! ```cargo
//! [dependencies]
//! turso = "0.8.3"
//! serde_json = "1"
//! tokio = { version = "1", features = ["full"] }
//! anyhow = "1"
//! ```

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};
use turso::{Builder, Connection};

const TOP_K: usize = 10;
const RRF_K: f64 = 60.0;
const INCREMENTAL_FILES: usize = 50;

struct Chunk {
    chunk_id: String,
    file_id: String,
    filepath: String,
    chunk_text: String,
    data: String,
    embedding: Vec<u8>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dataset, work, reps) = match args.as_slice() {
        [d, w] => (PathBuf::from(d), PathBuf::from(w), 20),
        [d, w, r] => (PathBuf::from(d), PathBuf::from(w), r.parse()?),
        _ => bail!("usage: bench_backend_turso.rs <dataset_dir> <work_dir> [reps]"),
    };
    let db_dir = work.join("turso");
    if db_dir.exists() {
        fs::remove_dir_all(&db_dir)?;
    }
    fs::create_dir_all(&db_dir)?;
    let db_path = db_dir.join("index.db");

    let chunks = read_chunks(&dataset.join("chunks.jsonl"))?;
    let queries = read_queries(&dataset.join("queries.jsonl"))?;

    // Load: rows first, then the FTS index (one backfill instead of a
    // segment per insert statement).
    let t = Instant::now();
    let mut conn = open(&db_path).await?;
    conn.execute_batch(
        "CREATE TABLE chunks (
             chunk_id TEXT PRIMARY KEY, file_id TEXT NOT NULL, filepath TEXT NOT NULL,
             chunk_text TEXT NOT NULL, data TEXT, embedding BLOB NOT NULL);
         CREATE INDEX chunks_file ON chunks (file_id);",
    )
    .await?;
    insert(&mut conn, chunks.iter()).await?;
    let insert_ms = ms(t);
    let t = Instant::now();
    run(&conn, "CREATE INDEX chunks_fts ON chunks USING fts (chunk_text)").await?;
    run(&conn, "OPTIMIZE INDEX chunks_fts").await?;
    let index_ms = ms(t);
    drop(conn);

    // Open.
    let t = Instant::now();
    let mut conn = open(&db_path).await?;
    let open_ms = ms(t);

    // Queries.
    let mut query_results = Vec::new();
    for q in &queries {
        run_query(&conn, q).await?;
        let mut times = Vec::with_capacity(reps);
        let mut top = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            top = run_query(&conn, q).await?;
            times.push(ms(t));
        }
        query_results.push(json!({
            "kind": q["kind"],
            "mode": q["mode"],
            "p50_ms": percentile(&mut times, 0.50),
            "p95_ms": percentile(&mut times, 0.95),
            "top_ids": top,
        }));
    }

    // Incremental: delete and re-add the chunks of the first N files.
    let file_ids: BTreeSet<&str> = chunks.iter().map(|c| c.file_id.as_str()).collect();
    let chosen: HashSet<&str> = file_ids.into_iter().take(INCREMENTAL_FILES).collect();
    let readd: Vec<&Chunk> = chunks
        .iter()
        .filter(|c| chosen.contains(c.file_id.as_str()))
        .collect();
    let in_list = chosen
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let t = Instant::now();
    conn.execute(&format!("DELETE FROM chunks WHERE file_id IN ({in_list})"), ())
        .await?;
    insert(&mut conn, readd.iter().copied()).await?;
    run(&conn, "OPTIMIZE INDEX chunks_fts").await?;
    let incremental_ms = ms(t);
    let count = query_i64(&conn, "SELECT count(*) FROM chunks").await?;
    if usize::try_from(count)? != chunks.len() {
        bail!("row count changed after the incremental update");
    }
    run(&conn, "PRAGMA wal_checkpoint(TRUNCATE)").await?;

    let result = json!({
        "backend": "turso",
        "version": "turso 0.8.3",
        "rows": chunks.len(),
        "insert_ms": insert_ms,
        "index_ms": index_ms,
        "open_ms": open_ms,
        "incremental_ms": incremental_ms,
        "incremental_rows": readd.len(),
        "disk_bytes": dir_size(&db_dir)?,
        "exe_bytes": fs::metadata(std::env::current_exe()?)?.len(),
        "reps": reps,
        "queries": query_results,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

async fn open(path: &Path) -> anyhow::Result<Connection> {
    let db = Builder::new_local(path.to_str().context("non-UTF8 path")?)
        .experimental_index_method(true)
        .build()
        .await?;
    Ok(db.connect()?)
}

/// Insert chunks in one transaction with a reused prepared statement.
async fn insert<'a>(
    conn: &mut Connection,
    chunks: impl Iterator<Item = &'a Chunk>,
) -> anyhow::Result<()> {
    let tx = conn.transaction().await?;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO chunks (chunk_id, file_id, filepath, chunk_text, data, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .await?;
        for c in chunks {
            stmt.execute((
                c.chunk_id.as_str(),
                c.file_id.as_str(),
                c.filepath.as_str(),
                c.chunk_text.as_str(),
                c.data.as_str(),
                c.embedding.as_slice(),
            ))
            .await?;
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Run one query and return the top chunk ids.
async fn run_query(conn: &Connection, q: &Value) -> anyhow::Result<Vec<String>> {
    let text = q["text"].as_str().context("query text")?;
    let embedding: Vec<f32> = serde_json::from_value(q["embedding"].clone())?;
    let blob = f32_blob(&embedding);
    let (filter, value) = match q["filter"].as_object() {
        Some(f) => (
            format!(
                "AND json_extract(data, '$.{}') = ?2",
                f["field"].as_str().unwrap_or_default()
            ),
            f["value"].as_str().unwrap_or_default().to_string(),
        ),
        None => ("AND ?2 IS NOT NULL".to_string(), String::new()),
    };
    let vector_sql = format!(
        "SELECT chunk_id FROM chunks WHERE 1 {filter}
         ORDER BY vector_distance_cos(embedding, ?1) LIMIT {TOP_K}"
    );
    // The FTS planner only pushes ORDER BY / LIMIT into the index when
    // fts_match is the whole WHERE clause.
    let fts_sql = if q["filter"].is_object() {
        format!(
            "SELECT chunk_id FROM chunks WHERE fts_match(chunk_text, ?1) {filter}
             ORDER BY fts_score(chunk_text, ?1) DESC LIMIT {TOP_K}"
        )
    } else {
        format!(
            "SELECT chunk_id, fts_score(chunk_text, ?1) AS score FROM chunks
             WHERE fts_match(chunk_text, ?1) ORDER BY score DESC LIMIT {TOP_K}"
        )
    };
    match q["mode"].as_str() {
        Some("semantic") => ids(conn, &vector_sql, (blob, value)).await,
        Some("fulltext") => fts_ids(conn, &fts_sql, text, &value, q["filter"].is_object()).await,
        Some("hybrid") => {
            let vector = ids(conn, &vector_sql, (blob, value.clone())).await?;
            let fts = fts_ids(conn, &fts_sql, text, &value, q["filter"].is_object()).await?;
            Ok(rrf(&[vector, fts]))
        }
        other => bail!("unknown mode {other:?}"),
    }
}

async fn fts_ids(
    conn: &Connection,
    sql: &str,
    text: &str,
    value: &str,
    filtered: bool,
) -> anyhow::Result<Vec<String>> {
    if filtered {
        ids(conn, sql, (text, value)).await
    } else {
        ids(conn, sql, (text,)).await
    }
}

async fn ids(
    conn: &Connection,
    sql: &str,
    params: impl turso::IntoParams,
) -> anyhow::Result<Vec<String>> {
    let mut rows = conn.query(sql, params).await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(row.get::<String>(0)?);
    }
    Ok(out)
}

/// Run a statement that may return rows, discarding them.
async fn run(conn: &Connection, sql: &str) -> anyhow::Result<()> {
    let mut rows = conn.query(sql, ()).await?;
    while rows.next().await?.is_some() {}
    Ok(())
}

async fn query_i64(conn: &Connection, sql: &str) -> anyhow::Result<i64> {
    let mut rows = conn.query(sql, ()).await?;
    let row = rows.next().await?.context("no row")?;
    Ok(row.get::<i64>(0)?)
}

/// Reciprocal rank fusion of ranked id lists.
fn rrf(lists: &[Vec<String>]) -> Vec<String> {
    let mut scores: HashMap<&str, f64> = HashMap::new();
    for list in lists {
        for (rank, id) in list.iter().enumerate() {
            *scores.entry(id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
    }
    let mut ranked: Vec<(&str, f64)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(b.0)));
    ranked
        .into_iter()
        .take(TOP_K)
        .map(|(id, _)| id.to_string())
        .collect()
}

fn read_chunks(path: &Path) -> anyhow::Result<Vec<Chunk>> {
    let mut out = Vec::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let v: Value = serde_json::from_str(&line?)?;
        let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
        let embedding: Vec<f32> = serde_json::from_value(v["embedding"].clone())?;
        out.push(Chunk {
            chunk_id: s("chunk_id"),
            file_id: s("file_id"),
            filepath: s("filepath"),
            chunk_text: s("chunk_text"),
            data: v.get("data").map_or_else(|| "{}".into(), Value::to_string),
            embedding: f32_blob(&embedding),
        });
    }
    if out.is_empty() {
        bail!("no chunks in {}", path.display());
    }
    Ok(out)
}

fn read_queries(path: &Path) -> anyhow::Result<Vec<Value>> {
    BufReader::new(File::open(path)?)
        .lines()
        .map(|l| Ok(serde_json::from_str(&l?)?))
        .collect()
}

fn f32_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn percentile(times: &mut [f64], p: f64) -> f64 {
    times.sort_by(f64::total_cmp);
    let i = ((times.len() as f64 - 1.0) * p).round() as usize;
    times[i]
}

fn dir_size(dir: &Path) -> anyhow::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let meta = entry?.metadata()?;
        total += meta.len();
    }
    Ok(total)
}
