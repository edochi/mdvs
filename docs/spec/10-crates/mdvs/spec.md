# Crate: `mdvs`

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Crate: mdvs-schema](../mdvs-schema/spec.md) | [Crate: mfv](../mfv/spec.md) | [Database Schema](../../20-database/schema.md)

---

## Overview

Full semantic search CLI binary (~20MB). Depends on `mdvs-schema` and `mfv`. Handles the complete pipeline: frontmatter extraction, semantic chunking, embedding, DuckDB storage, HNSW indexing, and cosine distance search.

**Responsibilities:**

- Initialize vault: create database, discover fields, prompt for promotion, download model
- Index files: incremental ingestion, chunking, embedding, HNSW index
- Search: embed query, cosine distance search, note-level ranking
- Model identity management and mismatch detection
- Delegate validation to `mfv` library
- Export data as Parquet
- Raw SQL passthrough

---

## CLI

```
mdvs <command> [options]

COMMANDS:
    init      Initialize a new index
    index     Build or update the index (incremental)
    reindex   Full rebuild (e.g., after model change)
    search    Semantic search across notes
    similar   Find notes similar to a given note
    validate  Validate frontmatter against schema
    query     Run raw SQL against indexed data
    export    Export database tables as Parquet
    info      Show index status and statistics
```

### Global Options

```
--db <path>       Path to .duckdb file (default: ./.mdvs.duckdb)
--dir <path>      Markdown directory root (default: .)
--model <id>      Override HuggingFace model ID
--revision <sha>  Override model revision
```

### `mdvs init`

```
mdvs init [--model <id>] [--revision <sha>] [--glob <pattern>]
          [--promoted <fields>] [--chunk-size <n>]
```

**Steps:**

1. Create `.mdvs.duckdb`
2. Install and load DuckDB `vss` extension
3. Scan frontmatter: discover fields via `mdvs_schema::discover_fields`
4. Present interactive promotion prompt (unless `--promoted` given)
5. Generate `frontmatter.toml` with promotion flags
6. Generate `.mdvs.toml` with model and chunking settings
7. Create database schema: `vault_meta`, `mdfiles` (with dynamic promoted columns), `chunks`
8. Download and cache embedding model via `model2vec-rs`
9. Store model identity in `vault_meta`

**Non-interactive mode:** `--promoted title,tags,date` skips the interactive prompt.

**If database already exists:** Error. User must delete it or use a different `--db` path.

### `mdvs index`

```
mdvs index [--full]
```

Incremental by default. See [Workflow: Index](../../30-workflows/index.md).

### `mdvs reindex`

```
mdvs reindex
```

Full rebuild. Nulls all embeddings, recomputes from stored `plain_text`. Updates model identity in `vault_meta`. Does not re-read files from disk or re-parse frontmatter.

### `mdvs search`

```
mdvs search <query> [--where <filter>] [-n <count>] [--format <fmt>] [--chunks]

Options:
    --where    SQL WHERE clause on mdfiles columns (e.g., "tags @> ['rust']")
    -n         Number of results (default: 10)
    --format   Output format: table (default), json, paths
    --chunks   Show chunk-level results instead of note-level grouping
```

See [Workflow: Search](../../30-workflows/search.md).

### `mdvs similar`

```
mdvs similar <file> [-n <count>]
```

Looks up stored embeddings for the given file, uses the average (or best) chunk embedding as query vector. No model inference needed — purely a database operation.

### `mdvs validate`

```
mdvs validate [--dir <path>] [--schema <path>]
```

Delegates to `mfv::validate`. Convenience command so users don't need a separate `mfv` binary.

### `mdvs query`

```
mdvs query <sql>
mdvs query -        # read SQL from stdin
```

Direct SQL access to the DuckDB database. Useful for ad-hoc analysis, debugging, custom reports.

### `mdvs export`

```
mdvs export [--output <dir>]
```

Exports tables as Parquet files:

```
mdvs-export/
├── mdfiles.parquet
├── chunks.parquet
└── vault_meta.parquet
```

### `mdvs info`

```
mdvs info
```

Displays: vault path, database size, file count, chunk count, model ID/dimension/revision, promoted fields, max chunk size, last indexed timestamp.

---

## Core Modules

### Ingestion Pipeline

```rust
mod ingest {
    /// Extract frontmatter + body from a markdown file
    fn parse_file(path: &Path) -> Result<(Option<Mapping>, String)>

    /// Split markdown body into semantic chunks
    fn chunk_markdown(body: &str, max_size: usize) -> Vec<Chunk>

    /// Extract plain text from a markdown chunk (strip syntax)
    fn extract_plain_text(markdown: &str) -> String

    /// Extract the nearest heading from a chunk's markdown
    fn extract_heading(markdown: &str) -> Option<String>

    /// Compute content hash for a file
    fn content_hash(content: &[u8]) -> String
}

struct Chunk {
    index: usize,
    markdown: String,
    plain_text: String,
    heading: Option<String>,
    char_count: usize,
}
```

### Embedding

```rust
mod embed {
    /// Load a Model2Vec model, returning model + resolved identity
    fn load_model(
        model_id: &str,
        revision: Option<&str>,
    ) -> Result<(Model, ModelIdentity)>

    /// Embed a batch of plain text strings
    fn embed_batch(
        model: &Model,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>>

    struct ModelIdentity {
        model_id: String,
        dimension: usize,
        revision: String,
    }
}
```

### Database

```rust
mod db {
    /// Create database and tables with dynamic promoted columns
    fn create_schema(
        conn: &Connection,
        promoted: &[FieldDef],
        dimension: usize,
    ) -> Result<()>

    /// Load model identity from vault_meta
    fn load_model_identity(conn: &Connection) -> Result<ModelIdentity>

    /// Store model identity to vault_meta
    fn store_model_identity(
        conn: &Connection,
        identity: &ModelIdentity,
    ) -> Result<()>

    /// Insert/update a file and its chunks
    fn upsert_file(
        conn: &Connection,
        file: &IndexedFile,
    ) -> Result<()>

    /// Delete a file and its chunks
    fn delete_file(conn: &Connection, filename: &str) -> Result<()>

    /// Semantic search with note-level ranking
    fn search(
        conn: &Connection,
        query_embedding: &[f32],
        where_clause: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SearchResult>>
}
```

### Model Mismatch

```rust
mod model {
    /// Check current model against stored identity
    fn check_identity(
        stored: &ModelIdentity,
        current: &ModelIdentity,
        operation: Operation,
    ) -> MismatchResult

    enum Operation { Index, Search, Similar }

    enum MismatchResult {
        Ok,
        Warning(String),
        Error(String),
    }
}
```

See [Workflow: Model Mismatch](../../30-workflows/model-mismatch.md).

---

## Staleness Behavior

Configured via `[behavior].on_search` in `.mdvs.toml`:

| Mode | Behavior |
|---|---|
| `auto` | Run incremental index sync before search (default) |
| `strict` | Error if any files have changed since last index |

In `auto` mode, `mdvs search` transparently runs the equivalent of `mdvs index` first, so results are always fresh. The overhead is minimal for unchanged vaults (just a hash comparison scan).

---

## Dependencies

| Crate | Purpose |
|---|---|
| `mdvs-schema` | Field definitions, type system, TOML parsing |
| `mfv` | Validation engine (library dependency) |
| `duckdb` (bundled) | Database, SQL, vector search host |
| `model2vec-rs` | Static embedding inference |
| `gray_matter` | Frontmatter extraction |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |
| `anyhow` | Error handling |
| `walkdir` | Filesystem traversal |
| `indicatif` | Progress bars |
| `xxhash-rust` or `blake3` | Content hashing |
| `serde_json` | JSON output format |

---

## Related Documents

- [Terminology](../../01-terminology.md)
- [Database Schema](../../20-database/schema.md) — table definitions and type mappings
- [Workflow: Init](../../30-workflows/init.md) — full init flow
- [Workflow: Index](../../30-workflows/index.md) — incremental indexing pipeline
- [Workflow: Search](../../30-workflows/search.md) — query, rank, display
- [Workflow: Model Mismatch](../../30-workflows/model-mismatch.md) — identity checks
- [Configuration: frontmatter.toml](../../40-configuration/frontmatter-toml.md)
- [Configuration: .mdvs.toml](../../40-configuration/mdvs-toml.md)
