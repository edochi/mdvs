# Database Schema

**Status: DRAFT**

**Cross-references:** [Terminology](../01-terminology.md) | [Crate: mdvs](../10-crates/mdvs/spec.md) | [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md)

---

## Overview

Storage is a single DuckDB file (`.mdvs.duckdb`) at the root of the vault. Three tables: `vault_meta` for configuration, `mdfiles` for file-level metadata, `chunks` for chunk text and embeddings. The `mdfiles` schema is dynamic — promoted columns are generated at init time based on user selection.

The DuckDB `vss` community extension is required for HNSW indexing and `array_cosine_distance()`.

---

## Tables

### `vault_meta`

Key-value store for index configuration. Queried on every operation to validate model identity and load settings.

```sql
CREATE TABLE vault_meta (
    key   VARCHAR PRIMARY KEY,
    value VARCHAR
);
```

**Stored keys:**

| Key | Example Value | Purpose |
|---|---|---|
| `model_id` | `minishlab/potion-multilingual-128M` | HuggingFace repo ID |
| `model_dimension` | `256` | Output vector size |
| `model_revision` | `a1b2c3d4e5f6` | Git commit SHA of model snapshot |
| `promoted_fields` | `["title","tags","date"]` | JSON array of promoted field names |
| `max_chunk_size` | `1000` | Maximum chunk size in characters |
| `vault_path` | `/home/user/notes` | Absolute path to vault root at index time |
| `glob_pattern` | `**` | File glob used for indexing |
| `created_at` | `2025-06-15T10:30:00Z` | When the index was first created |
| `last_indexed_at` | `2025-06-16T14:22:00Z` | When the last index/reindex completed |

### `mdfiles`

One row per markdown file. The schema is generated dynamically at init based on the user's promoted field selection.

```sql
CREATE TABLE mdfiles (
    filename      VARCHAR PRIMARY KEY,  -- relative path from vault root
    -- [dynamic promoted columns inserted here]
    metadata      JSON,                 -- all non-promoted frontmatter fields
    content_hash  VARCHAR,              -- hash of full file content
    indexed_at    TIMESTAMP DEFAULT current_timestamp
);
```

**Dynamic promoted columns** are inserted between `filename` and `metadata` based on the field schema. Example with `title`, `tags`, `date` promoted:

```sql
CREATE TABLE mdfiles (
    filename      VARCHAR PRIMARY KEY,
    title         VARCHAR,
    tags          VARCHAR[],
    date          DATE,
    metadata      JSON,
    content_hash  VARCHAR,
    indexed_at    TIMESTAMP DEFAULT current_timestamp
);
```

**Optional column:** When `store_raw_content = true` in `.mdvs.toml`, an additional `raw_content VARCHAR` column is added after `indexed_at`.

#### Type Mapping

Promoted fields map from `FieldType` (defined in `mdvs-schema`) to DuckDB column types:

| `FieldType` | DuckDB Type | Notes |
|---|---|---|
| `String` | `VARCHAR` | |
| `StringArray` | `VARCHAR[]` | |
| `Date` | `DATE` | |
| `Boolean` | `BOOLEAN` | |
| `Integer` | `BIGINT` | |
| `Float` | `DOUBLE` | |
| `Enum` | `VARCHAR` | No DB-level constraint; validated by `mfv` |

**NULL handling:** All promoted columns are nullable. Files without frontmatter, or with frontmatter missing a promoted field, get NULL for that column.

### `chunks`

One row per semantic chunk of a note.

```sql
CREATE TABLE chunks (
    chunk_id      VARCHAR PRIMARY KEY,  -- "{filename}#{chunk_index}"
    filename      VARCHAR NOT NULL REFERENCES mdfiles(filename),
    chunk_index   INTEGER,              -- 0-based position within the note
    heading       VARCHAR,              -- nearest heading ancestor (NULL if none)
    plain_text    VARCHAR,              -- markdown-stripped text content
    embedding     FLOAT[N],             -- N = model dimension (e.g., 256)
    char_count    INTEGER               -- character count of plain_text
);
```

**`chunk_id` format:** `"path/to/note.md#0"`, `"path/to/note.md#1"`, etc. Deterministic, human-readable, supports debugging.

**`heading`:** The nearest heading found within the chunk's markdown content. Used for the `§ Section Title` indicator in search results. NULL for chunks with no heading.

**`plain_text`:** Stored so that reindexing (after a model change) can recompute embeddings without re-reading files from disk.

**`embedding`:** `FLOAT[N]` where N is determined at init from the model's output dimension. Stored in `vault_meta.model_dimension`.

---

## HNSW Index

```sql
CREATE INDEX chunks_hnsw ON chunks USING HNSW (embedding)
    WITH (metric = 'cosine');
```

Created via the DuckDB `vss` community extension. Enables fast approximate nearest neighbor search.

**Rebuild:** The HNSW index is rebuilt after each `mdvs index` run. For typical vault sizes (< 50k chunks), this is fast.

---

## Extension Setup

```sql
INSTALL vss FROM community;
LOAD vss;
```

The `vss` extension is installed on first `mdvs init` and loaded on every database open. DuckDB caches extensions at `~/.duckdb/extensions/`.

---

## Dynamic Schema Generation

The `CREATE TABLE mdfiles` statement is generated at init by iterating over promoted fields from the field schema:

```
CREATE TABLE mdfiles (
    filename VARCHAR PRIMARY KEY,
    {for each promoted field: "{name} {duckdb_type},"}
    metadata JSON,
    content_hash VARCHAR,
    indexed_at TIMESTAMP DEFAULT current_timestamp
);
```

The promoted field list is persisted in `vault_meta.promoted_fields` as a JSON array. On subsequent runs, `mdvs` reads this to know which columns exist without re-parsing `frontmatter.toml`.

### Schema Changes

If the user re-runs `mdvs init` with different promoted fields, the database must be recreated. The existing database is incompatible because the `mdfiles` table has different columns. `mdvs init` refuses to overwrite an existing database — the user must delete it first.

---

## Query Patterns

### Semantic Search (note-level)

```sql
WITH ranked_chunks AS (
    SELECT
        c.filename,
        c.heading,
        LEFT(c.plain_text, :snippet_length) AS snippet,
        array_cosine_distance(c.embedding, :query_vec::FLOAT[N]) AS distance
    FROM chunks c
)
SELECT
    m.filename,
    -- [dynamic promoted columns]
    MIN(rc.distance) AS distance,
    FIRST(rc.heading ORDER BY rc.distance) AS best_heading,
    FIRST(rc.snippet ORDER BY rc.distance) AS snippet
FROM ranked_chunks rc
JOIN mdfiles m ON rc.filename = m.filename
-- [optional WHERE clause from --where flag]
GROUP BY m.filename -- [, dynamic promoted columns]
ORDER BY distance
LIMIT :limit;
```

The promoted column references in `SELECT` and `GROUP BY` are generated dynamically based on `vault_meta.promoted_fields`.

### Semantic Search (chunk-level, `--chunks`)

```sql
SELECT
    c.chunk_id,
    c.filename,
    c.heading,
    LEFT(c.plain_text, :snippet_length) AS snippet,
    array_cosine_distance(c.embedding, :query_vec::FLOAT[N]) AS distance
FROM chunks c
JOIN mdfiles m ON c.filename = m.filename
-- [optional WHERE clause]
ORDER BY distance
LIMIT :limit;
```

### Incremental Index Diff

```sql
-- Get existing hashes for comparison
SELECT filename, content_hash FROM mdfiles;
```

Compare against computed hashes from filesystem walk to determine new/modified/deleted/unchanged files.

---

## Related Documents

- [Terminology](../01-terminology.md) — definitions for vault_meta, mdfiles, chunks, HNSW, promoted field
- [Crate: mdvs](../10-crates/mdvs/spec.md) — `db` module that implements these operations
- [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md) — type mapping from `FieldType` to DuckDB types
- [Workflow: Index](../30-workflows/index.md) — how data flows into these tables
- [Workflow: Search](../30-workflows/search.md) — how queries execute against these tables
