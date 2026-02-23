mod chunk;
mod db;
mod embed;
mod frontmatter;
mod types;

use std::path::Path;

use anyhow::Result;
use mdvs_schema::FieldInfo;
use walkdir::WalkDir;

use types::NoteData;

const MODEL_ID: &str = "minishlab/potion-base-8M";
const CHUNK_MAX_CHARS: usize = 1000;
const PROMOTION_THRESHOLD: f64 = 0.5;
const SEARCH_LIMIT: usize = 5;

fn main() -> Result<()> {
    let target_dir = Path::new("tests/fixtures");
    let db_path = target_dir.join(".mdvs.duckdb");

    // Clean up any previous run
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(target_dir.join(".mdvs.duckdb.wal"));

    println!("=== mdvs v0.1 MVP ===\n");

    // 1. Scan directory
    println!("[1/6] Scanning {target_dir:?}...");
    let notes = scan_directory(target_dir)?;
    println!("      Found {} markdown files\n", notes.len());

    // 2. Discover fields
    println!("[2/6] Discovering frontmatter fields...");
    let frontmatters: Vec<Option<&serde_json::Value>> =
        notes.iter().map(|n| n.frontmatter.as_ref()).collect();
    let mut fields = mdvs_schema::discover_fields(&frontmatters);
    let total = notes.len();
    mdvs_schema::auto_promote(&mut fields, total, PROMOTION_THRESHOLD);
    print_field_table(&fields, total);

    // 3. Load model
    println!("[3/6] Loading model {MODEL_ID}...");
    let model = embed::load_model(MODEL_ID)?;
    let dim = embed::get_dimension(&model);
    println!("      Dimension: {dim}");
    if let Some(rev) = embed::resolve_revision(MODEL_ID) {
        println!("      Revision: {rev}");
    }
    println!();

    // 4. Create database
    println!("[4/6] Creating database at {db_path:?}...");
    let conn = db::open_db(&db_path)?;
    let promoted: Vec<&FieldInfo> = fields.iter().filter(|f| f.promoted).collect();
    db::create_tables(&conn, &promoted, dim)?;

    // Store model metadata
    db::store_meta(&conn, "model_id", MODEL_ID)?;
    db::store_meta(&conn, "dimension", &dim.to_string())?;
    if let Some(rev) = embed::resolve_revision(MODEL_ID) {
        db::store_meta(&conn, "model_revision", &rev)?;
    }
    println!();

    // 5. Process and embed each file
    println!("[5/6] Processing files...");
    for note in &notes {
        // Split frontmatter
        let (promoted_values, metadata) = if let Some(fm) = &note.frontmatter {
            frontmatter::split_frontmatter(fm, &fields)
        } else {
            (std::collections::HashMap::new(), serde_json::json!({}))
        };

        // Insert file record
        db::insert_file(
            &conn,
            &note.filename,
            &promoted,
            &promoted_values,
            &metadata,
            &note.content_hash,
        )?;

        // Chunk
        let chunks = chunk::chunk_note(&note.filename, &note.body, CHUNK_MAX_CHARS);

        // Embed
        let texts: Vec<String> = chunks.iter().map(|c| c.plain_text.clone()).collect();
        let embeddings = if texts.is_empty() {
            vec![]
        } else {
            embed::encode_batch(&model, &texts)
        };

        // Insert chunks
        db::insert_chunks(&conn, &chunks, &embeddings, dim)?;

        println!(
            "      {} — {} chunk(s), hash={}",
            note.filename,
            chunks.len(),
            &note.content_hash[..12]
        );
    }
    println!();

    // 6. Search
    let queries = [
        "how does CRDT conflict resolution work",
        "Nix flakes",
        "API design pagination",
    ];

    println!("[6/6] Running search queries...\n");
    for query in &queries {
        println!("--- Query: \"{query}\" ---");
        let query_emb = embed::encode_query(&model, query);
        let results = db::search(&conn, &query_emb, &promoted, dim, SEARCH_LIMIT)?;

        for (i, r) in results.iter().enumerate() {
            let heading = r.best_heading.as_deref().unwrap_or("-");
            let promoted_str: Vec<String> =
                r.promoted.iter().map(|(k, v)| format!("{k}={v}")).collect();
            let meta = if promoted_str.is_empty() {
                String::new()
            } else {
                format!(" [{}]", promoted_str.join(", "))
            };

            println!(
                "  {}. {} (dist={:.4}) §{}{}\n     {}",
                i + 1,
                r.filename,
                r.distance,
                heading,
                meta,
                r.snippet,
            );
        }
        println!();
    }

    println!("Done. Database at: {db_path:?}");
    Ok(())
}

fn scan_directory(dir: &Path) -> Result<Vec<NoteData>> {
    let mut notes = Vec::new();

    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            let content = std::fs::read_to_string(path)?;
            let filename = path
                .strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();

            let content_hash = format!("{:016x}", xxhash_rust::xxh3::xxh3_64(content.as_bytes()));
            let (fm, body) = mfv::extract_frontmatter(&content);

            notes.push(NoteData {
                filename,
                frontmatter: fm,
                body,
                content_hash,
            });
        }
    }

    notes.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(notes)
}

fn print_field_table(fields: &[FieldInfo], total: usize) {
    let header = "Promoted";
    println!(
        "      {:<20} {:<12} {:>5}/{:<5} {}",
        "Field", "Type", "Count", "Total", header
    );
    println!("      {}", "-".repeat(60));
    for f in fields {
        let marker = if f.promoted { "✓" } else { "" };
        println!(
            "      {:<20} {:<12} {:>5}/{:<5} {}",
            f.name, f.field_type, f.count, total, marker
        );
    }
    println!();
}
