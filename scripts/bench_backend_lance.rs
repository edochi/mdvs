#!/usr/bin/env rust-script
//! Backend benchmark: LanceDB, configured as mdvs uses it.
//!
//! Usage: `bench_backend_lance.rs <dataset_dir> <work_dir> [reps]`
//!
//! Loads `<dataset_dir>/chunks.jsonl` and `queries.jsonl` (written by
//! `bench_backend_export.rs`), runs the shared workload and prints one JSON
//! result on stdout. Workload: load (insert + FTS index + IVF-PQ cosine index
//! from 10,000 rows), open, each query `reps` times after one warm-up, then an
//! incremental update (delete and re-add the chunks of 50 files, optimize).
//!
//! ```cargo
//! [dependencies]
//! lancedb = "0.37"
//! lance-index = "=10.0.0"
//! arrow = { version = "58", features = ["json"] }
//! serde_json = "1"
//! tokio = { version = "1", features = ["full"] }
//! futures = "0.3"
//! anyhow = "1"
//! ```

use std::{
    collections::{BTreeSet, HashSet},
    fs::{self, File},
    io::{BufRead, BufReader, Seek},
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use anyhow::{Context, bail};
use arrow::{
    array::{
        Array, ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, ListArray, RecordBatch,
        RecordBatchIterator, RecordBatchReader, StringArray,
    },
    compute::filter_record_batch,
    datatypes::{DataType, Field, Schema, SchemaRef},
    json::{ReaderBuilder, reader::infer_json_schema_from_seekable},
};
use futures::TryStreamExt;
use lance_index::scalar::FullTextSearchQuery;
use lancedb::{
    DistanceType,
    connection::LanceFileVersion,
    database::listing::{ListingDatabaseOptions, NewTableConfig},
    index::{Index, scalar::FtsIndexBuilder, vector::IvfPqIndexBuilder},
    query::{ExecutableQuery, QueryBase, Select},
    table::{OptimizeAction, Table},
};
use serde_json::{Value, json};

const TABLE: &str = "index";
const TOP_K: usize = 10;
const VECTOR_INDEX_MIN_ROWS: usize = 10_000;
const INCREMENTAL_FILES: usize = 50;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (dataset, work, reps) = match args.as_slice() {
        [d, w] => (PathBuf::from(d), PathBuf::from(w), 20),
        [d, w, r] => (PathBuf::from(d), PathBuf::from(w), r.parse()?),
        _ => bail!("usage: bench_backend_lance.rs <dataset_dir> <work_dir> [reps]"),
    };
    let db_dir = work.join("lance");
    if db_dir.exists() {
        fs::remove_dir_all(&db_dir)?;
    }

    let batches = read_chunks(&dataset.join("chunks.jsonl"))?;
    let schema = batches[0].schema();
    let rows: usize = batches.iter().map(RecordBatch::num_rows).sum();
    let queries = read_queries(&dataset.join("queries.jsonl"))?;

    // Load.
    let t = Instant::now();
    let conn = connect(&db_dir).await?;
    let table = conn
        .create_table(TABLE, reader(batches.clone(), schema.clone()))
        .execute()
        .await?;
    let insert_ms = ms(t);
    let t = Instant::now();
    table
        .create_index(&["chunk_text"], Index::FTS(FtsIndexBuilder::default()))
        .execute()
        .await?;
    if rows >= VECTOR_INDEX_MIN_ROWS {
        table
            .create_index(
                &["embedding"],
                Index::IvfPq(IvfPqIndexBuilder::default().distance_type(DistanceType::Cosine)),
            )
            .execute()
            .await?;
    }
    let index_ms = ms(t);
    drop(table);
    drop(conn);

    // Open.
    let t = Instant::now();
    let conn = connect(&db_dir).await?;
    let table = conn.open_table(TABLE).execute().await?;
    let open_ms = ms(t);

    // Queries.
    let mut query_results = Vec::new();
    for q in &queries {
        run_query(&table, q).await?;
        let mut times = Vec::with_capacity(reps);
        let mut top = Vec::new();
        for _ in 0..reps {
            let t = Instant::now();
            top = run_query(&table, q).await?;
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
    let file_ids: BTreeSet<String> = batches
        .iter()
        .flat_map(|b| str_values(b, "file_id"))
        .collect();
    let chosen: HashSet<String> = file_ids.into_iter().take(INCREMENTAL_FILES).collect();
    let mut readd = Vec::new();
    for b in &batches {
        let mask: BooleanArray = str_values(b, "file_id")
            .map(|id| Some(chosen.contains(&id)))
            .collect();
        let part = filter_record_batch(b, &mask)?;
        if part.num_rows() > 0 {
            readd.push(part);
        }
    }
    let readd_rows: usize = readd.iter().map(RecordBatch::num_rows).sum();
    let in_list = chosen
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let t = Instant::now();
    table.delete(&format!("file_id IN ({in_list})")).await?;
    table
        .add(reader(readd, schema))
        .execute()
        .await?;
    table.optimize(OptimizeAction::All).await?;
    let incremental_ms = ms(t);
    if table.count_rows(None).await? != rows {
        bail!("row count changed after the incremental update");
    }

    let result = json!({
        "backend": "lancedb",
        "version": "lancedb 0.37 / lance 10",
        "rows": rows,
        "insert_ms": insert_ms,
        "index_ms": index_ms,
        "open_ms": open_ms,
        "incremental_ms": incremental_ms,
        "incremental_rows": readd_rows,
        "disk_bytes": dir_size(&db_dir)?,
        "exe_bytes": fs::metadata(std::env::current_exe()?)?.len(),
        "reps": reps,
        "queries": query_results,
    });
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn reader(batches: Vec<RecordBatch>, schema: SchemaRef) -> Box<dyn RecordBatchReader + Send> {
    Box::new(RecordBatchIterator::new(batches.into_iter().map(Ok), schema))
}

async fn connect(dir: &Path) -> anyhow::Result<lancedb::Connection> {
    // mdvs writes Lance v2.2 files.
    let options = ListingDatabaseOptions {
        new_table_config: NewTableConfig {
            data_storage_version: Some(LanceFileVersion::V2_2),
            ..Default::default()
        },
        ..Default::default()
    };
    Ok(lancedb::connect(dir.to_str().context("non-UTF8 path")?)
        .database_options(&options)
        .execute()
        .await?)
}

/// Run one query the way mdvs does and return the top chunk ids.
async fn run_query(table: &Table, q: &Value) -> anyhow::Result<Vec<String>> {
    let text = q["text"].as_str().context("query text")?;
    let embedding: Vec<f32> = serde_json::from_value(q["embedding"].clone())?;
    let filter = q["filter"].as_object().map(|f| {
        format!(
            "data.{} = '{}'",
            f["field"].as_str().unwrap_or_default(),
            f["value"].as_str().unwrap_or_default().replace('\'', "''")
        )
    });
    let projection = Select::columns(&["chunk_id"]);
    let fts = || FullTextSearchQuery::new(text.to_string());
    let batches: Vec<RecordBatch> = match q["mode"].as_str() {
        Some("semantic") => {
            let mut v = table
                .query()
                .select(projection)
                .nearest_to(embedding)?
                .distance_type(DistanceType::Cosine)
                .limit(TOP_K);
            if let Some(w) = &filter {
                v = v.only_if(w);
            }
            v.execute().await?.try_collect().await?
        }
        Some("hybrid") => {
            let mut v = table
                .query()
                .select(projection)
                .nearest_to(embedding)?
                .distance_type(DistanceType::Cosine)
                .full_text_search(fts())
                .limit(TOP_K);
            if let Some(w) = &filter {
                v = v.only_if(w);
            }
            v.execute().await?.try_collect().await?
        }
        Some("fulltext") => {
            let mut v = table
                .query()
                .select(projection)
                .full_text_search(fts())
                .limit(TOP_K);
            if let Some(w) = &filter {
                v = v.only_if(w);
            }
            v.execute().await?.try_collect().await?
        }
        other => bail!("unknown mode {other:?}"),
    };
    Ok(batches
        .iter()
        .filter(|b| b.num_rows() > 0)
        .flat_map(|b| str_values(b, "chunk_id"))
        .take(TOP_K)
        .collect())
}

/// Read `chunks.jsonl` into Arrow batches, with `embedding` as
/// `FixedSizeList<Float32>` (the type mdvs stores).
fn read_chunks(path: &Path) -> anyhow::Result<Vec<RecordBatch>> {
    let mut file = BufReader::new(File::open(path)?);
    let (inferred, _) = infer_json_schema_from_seekable(&mut file, None)?;
    file.rewind()?;
    let list_item = Arc::new(Field::new("item", DataType::Float32, true));
    let fields: Vec<Field> = inferred
        .fields()
        .iter()
        .map(|f| match f.name().as_str() {
            "embedding" => Field::new("embedding", DataType::List(list_item.clone()), false),
            _ => f.as_ref().clone(),
        })
        .collect();
    let reader = ReaderBuilder::new(Arc::new(Schema::new(fields)))
        .with_batch_size(4096)
        .build(file)?;

    let mut out = Vec::new();
    for batch in reader {
        let batch = batch?;
        let idx = batch.schema().index_of("embedding")?;
        let list = batch
            .column(idx)
            .as_any()
            .downcast_ref::<ListArray>()
            .context("embedding is not a list")?;
        let dim = list.value_length(0);
        let values = list
            .values()
            .as_any()
            .downcast_ref::<Float32Array>()
            .context("embedding values are not f32")?
            .clone();
        let fixed = FixedSizeListArray::try_new(list_item.clone(), dim, Arc::new(values), None)?;
        let mut fields: Vec<Field> = batch
            .schema()
            .fields()
            .iter()
            .map(|f| f.as_ref().clone())
            .collect();
        fields[idx] = Field::new("embedding", fixed.data_type().clone(), false);
        let mut columns: Vec<ArrayRef> = batch.columns().to_vec();
        columns[idx] = Arc::new(fixed);
        out.push(RecordBatch::try_new(Arc::new(Schema::new(fields)), columns)?);
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

fn str_values<'a>(batch: &'a RecordBatch, column: &str) -> impl Iterator<Item = String> + 'a {
    let col = batch
        .column_by_name(column)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>());
    col.into_iter()
        .flat_map(|c| c.iter().map(|v| v.unwrap_or_default().to_string()))
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
        let entry = entry?;
        let meta = entry.metadata()?;
        total += if meta.is_dir() {
            dir_size(&entry.path())?
        } else {
            meta.len()
        };
    }
    Ok(total)
}
