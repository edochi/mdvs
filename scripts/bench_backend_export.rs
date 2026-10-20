#!/usr/bin/env rust-script
//! Export a built mdvs index as the shared dataset for the backend benchmark
//! scripts (`bench_backend_*.rs`).
//!
//! Usage: `bench_backend_export.rs <vault> <queries.json> <out_dir>`
//!
//! Reads `<vault>/.mdvs/index.lance` and writes:
//! - `<out_dir>/chunks.jsonl`: one chunk per line with `chunk_id`, `file_id`,
//!   `filepath`, `chunk_text`, `data` (frontmatter object) and `embedding`.
//! - `<out_dir>/queries.jsonl`: each query from `<queries.json>` (a list of
//!   `{kind, text, mode, filter}`) plus its `embedding`, encoded with the
//!   model named in `<vault>/mdvs.toml`.
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.37"
//! arrow-array = "58"
//! arrow-json = "58"
//! model2vec-rs = "0.1.4"
//! serde_json = "1"
//! toml = "1"
//! tokio = { version = "1", features = ["full"] }
//! futures = "0.3"
//! anyhow = "1"
//! ```

use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::PathBuf,
};

use anyhow::{Context, bail};
use futures::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase, Select};
use model2vec_rs::model::StaticModel;
use serde_json::Value;

const COLUMNS: [&str; 6] = [
    "chunk_id",
    "file_id",
    "filepath",
    "chunk_text",
    "data",
    "embedding",
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [vault, queries, out] = args.as_slice() else {
        bail!("usage: bench_backend_export.rs <vault> <queries.json> <out_dir>");
    };
    let vault = PathBuf::from(vault);
    let out = PathBuf::from(out);
    fs::create_dir_all(&out)?;

    let db = vault.join(".mdvs");
    let conn = lancedb::connect(db.to_str().context("non-UTF8 path")?)
        .execute()
        .await?;
    let table = conn.open_table("index").execute().await?;
    let expected = table.count_rows(None).await?;
    let batches: Vec<_> = table
        .query()
        .select(Select::columns(&COLUMNS))
        .execute()
        .await?
        .try_collect()
        .await?;

    let mut writer = arrow_json::LineDelimitedWriter::new(BufWriter::new(File::create(
        out.join("chunks.jsonl"),
    )?));
    let mut rows = 0;
    for batch in &batches {
        writer.write(batch)?;
        rows += batch.num_rows();
    }
    writer.finish()?;
    if rows != expected {
        bail!("exported {rows} rows, table has {expected}");
    }

    let config: toml::Table = toml::from_str(&fs::read_to_string(vault.join("mdvs.toml"))?)?;
    let model_cfg = serde_json::to_value(
        config
            .get("embedding_model")
            .context("mdvs.toml has no [embedding_model]")?,
    )?;
    let model_name = model_cfg["name"].as_str().context("embedding_model.name")?;
    let revision = model_cfg.get("revision").and_then(Value::as_str);
    let model = StaticModel::from_pretrained(model_name, revision, None, None)?;

    let queries: Vec<Value> = serde_json::from_str(&fs::read_to_string(queries)?)?;
    let mut qw = BufWriter::new(File::create(out.join("queries.jsonl"))?);
    for mut q in queries {
        let text = q["text"].as_str().context("query without text")?.to_string();
        q["embedding"] = serde_json::to_value(model.encode_single(&text))?;
        serde_json::to_writer(&mut qw, &q)?;
        qw.write_all(b"\n")?;
    }
    qw.flush()?;

    eprintln!(
        "exported {rows} chunks and queries ({model_name}) to {}",
        out.display()
    );
    Ok(())
}
