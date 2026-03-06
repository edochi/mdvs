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

One row per markdown file. Fixed schema — all frontmatter is stored as a single JSON column.

| Column | Arrow Type | Description |
|---|---|---|
| `file_id` | `Utf8` | UUID v4 (primary key) |
| `filename` | `Utf8` | Relative path from vault root |
| `frontmatter` | `Utf8` | Full frontmatter as JSON string |
| `content_hash` | `Utf8` | xxh3 hash of full file content |
| `built_at` | `Timestamp(Microsecond, None)` | When this file was last processed |

**`file_id`:** UUID v4, generated at build time. Decouples identity from path — enables future rename detection without orphaning chunks.

**`frontmatter`:** JSON string containing all frontmatter fields. Files without frontmatter get NULL. Frontmatter filtering (e.g., `WHERE tags LIKE '%rust%'`) uses JSON extraction functions in DataFusion.

**`content_hash`:** Used for incremental builds. Compare against `mdvs.lock` `[[file]]` entries to detect changes.

### `chunks.parquet`

One row per semantic chunk of a note.

| Column | Arrow Type | Description |
|---|---|---|
| `chunk_id` | `Utf8` | UUID v4 |
| `file_id` | `Utf8` | FK to `files.parquet` |
| `chunk_index` | `Int32` | 0-based position within the file |
| `start_line` | `Int32` | Start line number in file (1-based) |
| `end_line` | `Int32` | End line number in file (1-based) |
| `embedding` | `FixedSizeList<Float32>(N)` | N = model dimension (e.g., 256) |

**`chunk_id`:** UUID v4. Stable identity that survives re-chunking — in future chunk-level hashing (v0.4+), unchanged chunks keep their ID and embedding even if their position shifts.

**`chunk_index`:** Positional ordering within the file. Recomputed on every rebuild. Not an identity — use `chunk_id` for stable references.

**`start_line` / `end_line`:** 1-based line numbers matching editor display. Used by `--snippets` to read chunk text directly from the original file. In rare cases where the splitter falls back to character-level splitting mid-line, two consecutive chunks may share a line number — this is acceptable.

**`embedding`:** `FixedSizeList<Float32>(N)` where N is determined at build time from the model's output dimension.

No `plain_text` stored — model changes require re-reading files from disk to re-chunk and re-embed. This keeps the artifact lightweight.

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
    MIN(c.distance) AS distance
FROM chunks_with_distance c
JOIN files f ON c.file_id = f.file_id
-- [optional: WHERE {user_provided_clause on f.frontmatter}]
GROUP BY f.filename
ORDER BY distance
LIMIT ?;
```

Default search output is ranked file paths only. With `--snippets`, the best-matching chunk's `start_line`/`end_line` are used to read text from the original file.

### Chunk-Level Search

```sql
SELECT
    c.chunk_id,
    f.filename,
    c.chunk_index,
    c.start_line,
    c.end_line,
    c.distance
FROM chunks_with_distance c
JOIN files f ON c.file_id = f.file_id
-- [optional: WHERE {user_provided_clause on f.frontmatter}]
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
- [Workflow: Build](../30-workflows/build.md) — how data flows into these files
- [Workflow: Search](../30-workflows/search.md) — how queries execute against these files
