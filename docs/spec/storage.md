# Storage

Deep-dive into the Parquet storage layer. For the module map see [architecture.md](./architecture.md).

The storage layer bridges validation (TOML config) and search (Parquet index). Key files: `index/storage.rs` (I/O, schema construction), `index/backend.rs` (backend abstraction).

## Two Artifacts

```
.mdvs/
  files.parquet     — one row per markdown file (manifest + frontmatter)
  chunks.parquet    — one row per chunk (text position + embedding vector)
```

Separate files because they have different lifecycles: `files.parquet` is rewritten on every build (fresh frontmatter from scan), while `chunks.parquet` is incrementally merged (retained chunks + new chunks).

## Column Layout

**files.parquet** — constants at `index/storage.rs:26-44`:

| Column | Constant | Arrow Type | Notes |
|--------|----------|-----------|-------|
| `file_id` | `COL_FILE_ID` | Utf8 | UUID, stable across incremental builds for unchanged files |
| `filepath` | `COL_FILEPATH` | Utf8 | Relative path from project root |
| `data` | `COL_DATA` | Struct | Children are frontmatter fields (see below) |
| `content_hash` | `COL_CONTENT_HASH` | Utf8 | xxh3-64 hex, body only |
| `built_at` | `COL_BUILT_AT` | Timestamp(Microsecond) | Build time |

**chunks.parquet**:

| Column | Constant | Arrow Type | Notes |
|--------|----------|-----------|-------|
| `chunk_id` | `COL_CHUNK_ID` | Utf8 | UUID |
| `file_id` | `COL_FILE_ID` | Utf8 | FK to files.parquet |
| `chunk_index` | `COL_CHUNK_INDEX` | Int32 | Zero-based within file |
| `start_line` | `COL_START_LINE` | Int32 | 1-based in source |
| `end_line` | `COL_END_LINE` | Int32 | 1-based, inclusive |
| `embedding` | `COL_EMBEDDING` | FixedSizeList(Float32) | Dimension from model |

All column names are prefixed with `internal_prefix` (default `_`) at write time via `col(prefix, name)` helper.

## The `data` Struct Column

The most complex construction in storage. `build_files_batch()` at `index/storage.rs:260` builds the `data` column as a nested Arrow Struct whose children match the field definitions in `mdvs.toml`.

For each `(name, FieldType)` pair in `schema_fields`:
1. Extract the value from each file's frontmatter JSON: `file.frontmatter.get(name)`
2. Call `build_array(values, field_type)` to construct the typed Arrow array

`build_array()` handles the FieldType→Arrow mapping recursively:

| FieldType | Arrow Array | Conversion |
|-----------|------------|------------|
| Boolean | BooleanArray | `v.as_bool()` |
| Integer | Int64Array | `v.as_i64()` |
| Float | Float64Array | `v.as_f64()`, falls back to `v.as_i64() as f64` |
| String | StringArray | actual strings preserved; non-strings serialized to JSON repr |
| Array(inner) | ListArray | variable-length, child built recursively via `build_array` |
| Object(fields) | StructArray | nested Struct, children built recursively |

**String is special**: the "top type" guarantee means a String-typed field can hold any JSON value. Non-string values (`true`, `42`, `["a"]`) are serialized to their JSON string representation, never dropped as NULL. This is the "never silently drop data" contract.

## Content Hash

`content_hash()` at `index/storage.rs:70`:

```rust
pub fn content_hash(content: &str) -> String {
    format!("{:016x}", xxh3_64(content.as_bytes()))
}
```

- Input: markdown body only (after frontmatter extraction by `gray_matter`)
- Algorithm: xxHash3-64
- Output: 16-character hex string

Frontmatter-only changes (editing a `status` field) do NOT trigger re-embedding. The hash covers the body that gets chunked and embedded.

## Build Metadata

`BuildMetadata` at `index/storage.rs:109` stores the build configuration snapshot:

| Key | Source |
|-----|--------|
| `mdvs.provider` | `EmbeddingModelConfig.provider` |
| `mdvs.model` | `EmbeddingModelConfig.name` |
| `mdvs.revision` | `EmbeddingModelConfig.revision` |
| `mdvs.chunk_size` | `ChunkingConfig.max_chunk_size` |
| `mdvs.glob` | `ScanConfig.glob` |
| `mdvs.built_at` | ISO 8601 timestamp |

Stored in Parquet native key-value metadata on `files.parquet` only (not `chunks.parquet`). Written via `write_parquet_with_metadata()`, read via `read_build_metadata()`.

**Config change detection**: build compares current config against stored `BuildMetadata` using `PartialEq`. Mismatch → requires `--force` for full rebuild. Search compares model identity → hard error on mismatch.

## Incremental Build

### Classification

`FileIndexEntry` at `index/storage.rs:432` is a lightweight projected read (columns 0, 1, 3 only — skips the expensive `data` Struct column):

```rust
pub struct FileIndexEntry {
    pub file_id: String,
    pub filename: String,
    pub content_hash: String,
}
```

Classification in `cmd/build.rs` compares scanned files against the index:

| Classification | Condition | Action |
|---------------|-----------|--------|
| **New** | filename not in index | Generate new file_id, chunk, embed |
| **Edited** | filename in index, hash differs | Keep file_id, re-chunk, re-embed |
| **Unchanged** | filename in index, hash matches | Skip chunking/embedding, retain existing chunks |
| **Removed** | in index, not in scan | Drop from output |

### Merge Strategy

1. Read retained chunks from existing `chunks.parquet` via `read_chunk_rows()` — filtered to only file_ids of unchanged files
2. Chunk and embed new + edited files
3. Combine retained chunks with new chunks
4. Write both parquet files from scratch (full rewrite, not append)

Model loading is skipped entirely when `needs_embedding == 0` (all files unchanged).

## Backend Abstraction

`Backend` enum at `index/backend.rs:41` wraps `ParquetBackend` (single variant). Constructor: `Backend::parquet(root, prefix)`.

`ParquetBackend` derives paths from root:
- `.mdvs/` — index directory
- `.mdvs/files.parquet` — file manifest
- `.mdvs/chunks.parquet` — chunk embeddings

Key methods: `write_index()` (orchestrates both parquet writes), `read_build_metadata()`, `read_file_index()`, `read_chunk_rows()`, `search()` (creates `SearchContext` and executes query), `index_stats()`, `exists()`.

Future: `Backend::Lance(LanceBackend)` variant planned for ANN indexing (TODO-0016).
