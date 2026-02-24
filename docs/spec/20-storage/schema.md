# Storage Schema

**Status: DRAFT**

**Cross-references:** [Terminology](../01-terminology.md) | [Crate: mdvs](../10-crates/mdvs/spec.md) | [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md)

---

## Overview

The mdvs artifact is a `.mdvs/` directory at the root of the vault, containing compressed Parquet files. DataFusion (pure Rust SQL engine on Arrow) registers these files as tables for querying. The directory is co-located with the data, portable (move the vault, the index follows), and `.gitignore`-able.

Artifact metadata (model identity, build timestamps) lives in `mdvs.lock`, not in `.mdvs/`. The artifact directory contains only data files.

---

## Artifact Directory

```
vault/
├── mdvs.toml          # config (boundaries)
├── mdvs.lock          # resolved state (field observations + file hashes + build metadata)
├── .mdvs/             # artifact (searchable index)
│   ├── files.parquet  # one row per markdown file
│   └── chunks.parquet # one row per semantic chunk (with embeddings)
└── ... .md files ...
```

---

## Parquet Files

### `files.parquet`

One row per markdown file. The schema is generated dynamically at build time based on the field definitions in `mdvs.toml`.

**Fixed columns:**

| Column | Arrow Type | Description |
|---|---|---|
| `filename` | `Utf8` | Relative path from vault root (primary key) |
| `metadata` | `Utf8` | JSON string of all frontmatter fields not in the schema |
| `content_hash` | `Utf8` | Hash of full file content |
| `built_at` | `Timestamp(Microsecond, None)` | When this file was last processed |

**Dynamic field columns** are inserted between `filename` and `metadata` based on the field schema. Example with `title`, `tags`, `date` in the schema:

| Column | Arrow Type |
|---|---|
| `filename` | `Utf8` |
| `title` | `Utf8` |
| `tags` | `List<Utf8>` |
| `date` | `Date32` |
| `metadata` | `Utf8` |
| `content_hash` | `Utf8` |
| `built_at` | `Timestamp(Microsecond, None)` |

**NULL handling:** All field columns are nullable. Files without frontmatter, or with frontmatter missing a schema field, get NULL for that column.

### `chunks.parquet`

One row per semantic chunk of a note.

| Column | Arrow Type | Description |
|---|---|---|
| `chunk_id` | `Utf8` | `"{filename}#{chunk_index}"` (e.g., `"notes/idea.md#0"`) |
| `filename` | `Utf8` | Parent file (FK to `files.parquet`) |
| `chunk_index` | `Int32` | 0-based position within the note |
| `heading` | `Utf8` (nullable) | Nearest heading ancestor in the chunk's markdown |
| `plain_text` | `Utf8` | Markdown-stripped text content |
| `embedding` | `FixedSizeList<Float32>(N)` | N = model dimension (e.g., 256) |
| `char_count` | `Int32` | Character count of `plain_text` |

**`chunk_id` format:** `"path/to/note.md#0"`, `"path/to/note.md#1"`, etc. Deterministic, human-readable.

**`heading`:** Used for the `§ Section Title` indicator in search results. NULL for chunks with no heading.

**`plain_text`:** Stored so that rebuilding (after a model change) can recompute embeddings without re-reading files from disk.

**`embedding`:** `FixedSizeList<Float32>(N)` where N is determined at init from the model's output dimension.

---

## Type Mapping

Field types from `mdvs-schema` map to Arrow types:

| `FieldType` | Arrow Type | Notes |
|---|---|---|
| `String` | `Utf8` | |
| `StringArray` | `List<Utf8>` | |
| `Date` | `Date32` | |
| `Boolean` | `Boolean` | |
| `Integer` | `Int64` | |
| `Float` | `Float64` | |
| `Enum` | `Utf8` | No Arrow-level constraint; validated by `mfv`/`mdvs check` |

---

## Compression

Parquet files use built-in compression. The default is snappy (fast, reasonable ratio). Zstd is an alternative for better compression at slightly higher CPU cost. The compression codec is an implementation detail, not user-configurable.

---

## Query Patterns

DataFusion registers both Parquet files as tables via `SessionContext`:

```rust
ctx.register_parquet("files", ".mdvs/files.parquet", opts).await?;
ctx.register_parquet("chunks", ".mdvs/chunks.parquet", opts).await?;
```

### Cosine Distance

Cosine distance is **not** computed via SQL. Instead:

1. Load `chunks.parquet` embedding column as an Arrow `FixedSizeList<Float32>` array
2. Compute cosine distance in Rust (vectorized over the Arrow array)
3. Append the distance as a new `Float64` column to the RecordBatch
4. Register the enriched RecordBatch as a DataFusion table
5. Use DataFusion SQL for JOIN, GROUP BY, ORDER BY, LIMIT, WHERE

This avoids the complexity of DataFusion UDFs while keeping the hot path (distance computation) in pure Rust.

### Note-Level Ranking

```sql
SELECT
    f.filename,
    -- [dynamic field columns]
    MIN(c.distance) AS distance,
    FIRST_VALUE(c.heading ORDER BY c.distance) AS best_heading,
    FIRST_VALUE(c.snippet ORDER BY c.distance) AS snippet
FROM chunks_with_distance c
JOIN files f ON c.filename = f.filename
-- [optional: WHERE {user_provided_clause}]
GROUP BY f.filename -- [, dynamic field columns]
ORDER BY distance
LIMIT ?;
```

### Chunk-Level Search

```sql
SELECT
    c.chunk_id,
    c.filename,
    c.heading,
    c.snippet,
    c.distance
FROM chunks_with_distance c
JOIN files f ON c.filename = f.filename
-- [optional: WHERE {user_provided_clause}]
ORDER BY c.distance
LIMIT ?;
```

### Incremental Build Diff

Content hashes are stored in `mdvs.lock` `[[file]]` entries. The build pipeline compares these against freshly computed hashes from the filesystem to determine new/modified/deleted/unchanged files.

---

## Vector Search

No ANN index initially — search performs brute-force cosine distance over all chunks. For typical vault sizes (< 50k chunks), this is fast enough. `hnsw_rs` is a future upgrade path for larger vaults.

---

## Related Documents

- [Terminology](../01-terminology.md) — definitions for artifact, chunk, embedding, build
- [Crate: mdvs](../10-crates/mdvs/spec.md) — `storage` module that implements Parquet I/O
- [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md) — type mapping from `FieldType` to Arrow types
- [Workflow: Build](../30-workflows/build.md) — how data flows into these files
- [Workflow: Search](../30-workflows/search.md) — how queries execute against these files
