//! Probe LanceDB's incremental-write surface against a real mdvs index.
//! Always runs against a COPY of the source index in a tempdir, so the
//! original is never mutated.
//!
//!     cargo run --release --example probe_lance_incremental -- <vault-path>
//!
//! What it checks:
//!   1. Schema metadata mutation on a live table (`replace_schema_metadata`,
//!      `update_config`, `delete_config_keys`).
//!   2. Row shape — one row per chunk with file-level columns repeated, or
//!      separate file rows?
//!   3. Delete API behavior + cost.
//!   4. Optimize cost on a 22k-chunk K8s-scale table.

use anyhow::Context;
use arrow::array::{Array, RecordBatch, StringArray};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use lancedb::table::OptimizeAction;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

const LANCE_TABLE: &str = "index";

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let vault = std::env::args()
        .nth(1)
        .context("usage: probe_lance_incremental <vault-path>")?;
    let lance_src = PathBuf::from(&vault).join(".mdvs").join("index.lance");
    if !lance_src.exists() {
        anyhow::bail!(
            "no Lance index at {} — run `mdvs build {}` first",
            lance_src.display(),
            vault
        );
    }

    let tmp = tempfile::tempdir()?;
    // mdvs connects at the .mdvs directory; the table dir
    // `.mdvs/index.lance/` is what `open_table("index")` resolves to.
    // So we replicate that layout in the tempdir.
    let lance_dst = tmp.path().join("index.lance");
    let t = Instant::now();
    copy_dir(&lance_src, &lance_dst)?;
    let copy_ms = t.elapsed().as_millis();
    println!("vault        : {vault}");
    println!(
        "source index : {} (KB: {})",
        lance_src.display(),
        dir_size_kb(&lance_src)
    );
    println!(
        "working copy : {} (copy took {copy_ms} ms, size: {} KB)",
        lance_dst.display(),
        dir_size_kb(&lance_dst)
    );
    println!();

    // Connect at the parent (tempdir root), open the "index" table.
    let uri = tmp.path().to_str().context("non-utf8 tempdir path")?;
    let conn = lancedb::connect(uri).execute().await?;
    let table = conn.open_table(LANCE_TABLE).execute().await?;

    // ---- (2) Row shape ----
    println!("== row shape ==");
    let schema = table.schema().await?;
    println!("schema fields ({}):", schema.fields().len());
    for f in schema.fields() {
        println!("  - {:<20} {}", f.name(), f.data_type());
    }
    let count = table.count_rows(None).await?;
    println!("row count           : {count}");
    println!();

    // ---- (1) Schema metadata ----
    println!("== schema metadata (the `mdvs.*` BuildMetadata keys) ==");
    let md = schema.metadata();
    let mut sorted: Vec<(&String, &String)> = md.iter().collect();
    sorted.sort_by_key(|(k, _)| (*k).clone());
    for (k, v) in &sorted {
        // Truncate long values (some may be JSON blobs).
        let v_preview = if v.len() > 80 { &v[..80] } else { v.as_str() };
        println!("  {k:<32} = {v_preview}");
    }
    println!();

    // Try replace_schema_metadata with a tweaked map (bumping built_at).
    let new_md: HashMap<String, String> = md
        .iter()
        .map(|(k, v)| {
            let nv = if k == "mdvs.built_at" {
                chrono::Utc::now().to_rfc3339()
            } else {
                v.clone()
            };
            (k.clone(), nv)
        })
        .collect();
    // `replace_schema_metadata` lives on `NativeTable`, not the public `Table` —
    // call via `as_native()`. If we ever ship cloud/remote Tables this becomes
    // a downcast guard; for the local-file backend it just returns Some(_).
    let native = table
        .as_native()
        .context("table is not a NativeTable — incremental probe assumes local backend")?;
    let t = Instant::now();
    let res = native.replace_schema_metadata(new_md).await;
    let md_ms = t.elapsed().as_millis();
    match res {
        Ok(()) => println!("replace_schema_metadata: OK ({md_ms} ms)"),
        Err(e) => println!("replace_schema_metadata: ERR ({md_ms} ms) -> {e}"),
    }
    // Re-read and confirm the change took.
    let schema2 = table.schema().await?;
    let new_built_at = schema2
        .metadata()
        .get("mdvs.built_at")
        .cloned()
        .unwrap_or_else(|| "<missing>".to_string());
    println!("  after replace, mdvs.built_at = {new_built_at}");
    println!();

    // ---- (3) Delete behavior ----
    println!("== delete ==");

    // Empty no-op (no match).
    let t = Instant::now();
    let _ = table.delete("file_id = '__nonexistent_sentinel__'").await?;
    let dt_ms = t.elapsed().as_millis();
    println!("no-op delete   : {dt_ms} ms");

    // Real delete: pick one file_id from the table, delete its rows.
    let batches: Vec<RecordBatch> = table
        .query()
        .select(Select::columns(&["file_id".to_string()]))
        .limit(1)
        .execute()
        .await?
        .try_collect()
        .await?;
    let mut victim_id: Option<String> = None;
    for batch in &batches {
        let col = batch.column_by_name("file_id").context("file_id column")?;
        let arr = col
            .as_any()
            .downcast_ref::<StringArray>()
            .context("file_id is not StringArray")?;
        if arr.len() > 0 {
            victim_id = Some(arr.value(0).to_string());
            break;
        }
    }
    if let Some(id) = victim_id {
        let pred = format!("file_id = '{id}'");
        let pre = table.count_rows(Some(pred.clone())).await?;
        let t = Instant::now();
        let _ = table.delete(&pred).await?;
        let real_ms = t.elapsed().as_millis();
        let post = table.count_rows(Some(pred)).await?;
        println!("real delete (1 id)  : {real_ms} ms (rows matching pre: {pre}, post: {post})");
    }

    // Batch delete cost: does `IN (...)` scale with list size?
    // Pull a chunk of distinct file_ids and time deletes for 1, 10, 50, 200.
    let id_batches: Vec<RecordBatch> = table
        .query()
        .select(Select::columns(&["file_id".to_string()]))
        .limit(400)
        .execute()
        .await?
        .try_collect()
        .await?;
    let mut sample_ids: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for b in &id_batches {
        let arr = b
            .column_by_name("file_id")
            .context("file_id column")?
            .as_any()
            .downcast_ref::<StringArray>()
            .context("file_id is not StringArray")?;
        for i in 0..arr.len() {
            let v = arr.value(i).to_string();
            if seen.insert(v.clone()) {
                sample_ids.push(v);
            }
        }
    }
    println!(
        "collected {} distinct file_ids for batch-delete sweep",
        sample_ids.len()
    );
    for n in [1usize, 10, 50, 200] {
        if sample_ids.len() < n {
            continue;
        }
        let slice = &sample_ids[..n];
        let in_list = slice
            .iter()
            .map(|id| format!("'{id}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let pred = format!("file_id IN ({in_list})");
        let t = Instant::now();
        let _ = table.delete(&pred).await?;
        let ms = t.elapsed().as_millis();
        println!("batch delete ({n:>3} ids) : {ms} ms");
    }
    println!();

    // ---- (4) Optimize cost ----
    println!("== optimize ==");
    let t = Instant::now();
    let stats = table.optimize(OptimizeAction::All).await?;
    let opt_ms = t.elapsed().as_millis();
    println!("optimize(All)  : {opt_ms} ms");
    println!("  compaction_metrics: {:?}", stats.compaction);
    println!("  prune_metrics     : {:?}", stats.prune);

    // Row count + size after.
    let final_count = table.count_rows(None).await?;
    let final_size = dir_size_kb(&lance_dst);
    println!();
    println!("post-probe row count: {final_count} (started: {count})");
    println!("post-probe index KB : {final_size}");

    Ok(())
}

fn dir_size_kb(p: &Path) -> u64 {
    let mut total: u64 = 0;
    fn walk(p: &Path, total: &mut u64) {
        let Ok(entries) = std::fs::read_dir(p) else {
            return;
        };
        for e in entries.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                walk(&e.path(), total);
            } else if let Ok(meta) = e.metadata() {
                *total += meta.len();
            }
        }
    }
    walk(p, &mut total);
    total / 1024
}
