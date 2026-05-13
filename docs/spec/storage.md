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

The `data` column is a nested Arrow Struct whose children mirror the YAML's natural shape: a YAML key like `calibration.baseline.wavelength` lands inside a `calibration` Struct child that holds a `baseline` Struct child holding a `wavelength` Float leaf. This shape lets DataFusion handle `WHERE calibration.baseline.wavelength > 800` natively via struct field access.

`build_files_batch()` at `index/storage.rs` produces this shape in two steps (post Wave C / TODO-0097):

1. **Transpose** the flat list of dotted-name `(name, FieldType)` entries from `mdvs.toml` into a synthetic `FieldType::Object` tree via `transpose_to_storage_type`. This reconstructs the canonical schema's natural shape.
2. **Recurse** via `build_array` against the synthesized tree, passing each file's whole frontmatter Value as the per-row input. The existing Object arm walks `properties.cal.properties.baseline.properties.wavelength` and assembles the corresponding nested `StructArray` columns.

`build_array()` handles the FieldType→Arrow mapping recursively:

| FieldType | Arrow Array | Conversion |
|-----------|------------|------------|
| Boolean | BooleanArray | `v.as_bool()` |
| Integer | Int64Array | `v.as_i64()` |
| Float | Float64Array | `v.as_f64()`, falls back to `v.as_i64() as f64` |
| String | StringArray | actual strings preserved; non-strings serialized to JSON repr |
| Array(inner) | ListArray | variable-length, child built recursively via `build_array` |
| Object(fields) | StructArray | nested Struct, children built recursively. Reached only via the synthesized storage tree's intermediates (Wave C transposes flat dotted-name leaves back into a nested Object before Arrow encoding). `Array(Object{...})` is rejected on the disk surface (TODO-0155), so no on-disk type produces this arm directly. |

**Per-row validity** follows the data: a file with `calibration: null` (or no `calibration` key) sees the `cal` Struct column's validity bit set to 0 for that row, propagating to all descendant columns. A file with `calibration: {baseline: {intensity: 0.5}}` but no `wavelength` leaf sees the leaf's validity bit set to 0 while the intermediate Structs are valid.

**String preprocessing**: a `String` field is strict by default — non-string JSON values violate validation and never reach the storage layer. Fields declaring `preprocess = ["coerce_to_string"]` (often auto-inferred when mixed types were observed) accept any JSON value; non-strings are serialized to their JSON string representation before validation, then stored as strings. This preserves the "never silently drop data" contract for fields that opt in.

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
| `mdvs.schema_hash` | xxh3-64 hex of `dsl_to_canonical(config)` serialized as canonical JSON |

Stored in Parquet native key-value metadata on `files.parquet` only (not `chunks.parquet`). Written via `write_parquet_with_metadata()`, read via `read_build_metadata()`.

**Schema hash** detects field-level changes (types, constraints, path-scoping, preprocessors) that don't show up in any of the other keys. Computed via `compute_schema_hash(config)` in `index/storage.rs`. Hashing the post-translation canonical JSON makes it whitespace-insensitive and key-order-insensitive. Pre-Wave-B parquets without this key read as `""` → treated as changed (conservative, requires `--force`).

**Config change detection**: build compares current config against stored `BuildMetadata` using `PartialEq`. Mismatch → requires `--force` for full rebuild. The schema-hash mismatch error reads: `"schema: fields, types, constraints, path-scoping, or preprocessors have changed"`. Search compares model identity → hard error on mismatch.

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
