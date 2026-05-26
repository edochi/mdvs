# Storage

Deep-dive into the Lance storage layer. For the module map see [architecture.md](./architecture.md).

The storage layer bridges validation (TOML config) and search (LanceDB index). Key files: `index/storage.rs` (Arrow batch construction, column constants, `BuildMetadata`, `content_hash`), `index/backend.rs` (`LanceBackend`: connection, `write_index`, `search`, `--where` translator).

## One Artifact

```
.mdvs/
  index.lance/      — Lance dataset, one row per chunk, plus FTS + (optional) vector indexes
```

A single Lance dataset (table name `index`) holds everything. Each row corresponds to one chunk; per-file fields (`filepath`, `data`, `content_hash`, `built_at`) are duplicated onto each of that file's chunk rows. There is no separate manifest file: per-build configuration lives as table-level key-value metadata on the dataset.

This collapsed layout replaces the earlier two-file design (`files.parquet` + `chunks.parquet`) — Lance's single-table-with-indexes model is the more idiomatic fit, and persisting `chunk_text` on the row makes both BM25 full-text indexing and verbose-mode snippet display trivial.

## Column Layout

Column constants live at the top of `index/storage.rs`:

| Column | Constant | Arrow Type | Notes |
|---|---|---|---|
| `chunk_id` | `COL_CHUNK_ID` | Utf8 | UUID |
| `file_id` | `COL_FILE_ID` | Utf8 | UUID, stable across incremental builds for unchanged files |
| `chunk_index` | `COL_CHUNK_INDEX` | Int32 | Zero-based within file |
| `start_line` | `COL_START_LINE` | Int32 | 1-based in source |
| `end_line` | `COL_END_LINE` | Int32 | 1-based, inclusive |
| `chunk_text` | `COL_CHUNK_TEXT` | Utf8 | The plain-text chunk body. Persisted so the BM25 index can tokenize it and verbose snippets can read it without a second file open. |
| `embedding` | `COL_EMBEDDING` | FixedSizeList<Float32, dim> | Dimension from the active embedding model |
| `filepath` | `COL_FILEPATH` | Utf8 | Relative path from project root (duplicated per chunk) |
| `content_hash` | `COL_CONTENT_HASH` | Utf8 | xxh3-64 hex of the markdown body (duplicated per chunk) |
| `data` | `COL_DATA` | Struct | Frontmatter as a nested Struct (see below) |
| `built_at` | `COL_BUILT_AT` | Timestamp(Microsecond, UTC) | Build time |

On-disk column names are always the literal constants above. `[search].internal_prefix` and `[search.aliases]` only affect how bare names in `--where` clauses are resolved by the translator (see [search.md](./search.md#collision-avoidance)), not what gets written to disk.

## Indexes Inside the Dataset

`LanceBackend::build_indexes()` runs after the table is written:

- **FTS (BM25)** on `chunk_text` — always built. Powers `--mode fulltext` and the hybrid mode's lexical leg.
- **Cosine IVF-PQ** on `embedding` — built only when `n_chunks >= VECTOR_INDEX_MIN_ROWS = 10_000`. Smaller vaults rely on LanceDB's exact flat scan via `nearest_to`, which is plenty fast at that scale.

## The `data` Struct Column

The `data` column is a nested Arrow Struct whose children mirror the source frontmatter's natural shape (YAML mapping, TOML table, or JSON object — all three deserialize to the same JSON shape, which the storage layer transposes into Arrow). A key like `calibration.baseline.wavelength` lands inside a `calibration` Struct child that holds a `baseline` Struct child holding a `wavelength` Float leaf. This lets LanceDB's SQL filter handle `data.calibration.baseline.wavelength > 800` natively via struct field access.

`build_files_batch()` in `index/storage.rs` produces this shape in two steps (post Wave C / TODO-0097):

1. **Transpose** the flat list of dotted-name `(name, FieldType)` entries from `mdvs.toml` into a synthetic `FieldType::Object` tree via `transpose_to_storage_type`. This reconstructs the canonical schema's natural shape.
2. **Recurse** via `build_array` against the synthesized tree, passing each file's whole frontmatter Value as the per-row input. The existing Object arm walks `properties.calibration.properties.baseline.properties.wavelength` and assembles the corresponding nested `StructArray` columns.

`build_array()` handles the FieldType→Arrow mapping recursively:

| FieldType | Arrow Array | Conversion |
|---|---|---|
| Boolean | BooleanArray | `v.as_bool()` |
| Integer | Int64Array | `v.as_i64()` |
| Float | Float64Array | `v.as_f64()`, falls back to `v.as_i64() as f64` |
| String | StringArray | actual strings preserved; non-strings serialized to JSON repr |
| Date | Date32Array | `chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")`, encoded as days since 1970-01-01. Unparseable values → NULL (defensive; jsonschema's `format: date` already rejects them upstream) |
| DateTime | TimestampMillisecondArray (tz = "UTC") | `chrono::DateTime::parse_from_rfc3339(s)` → `with_timezone(&Utc).timestamp_millis()`. Offsets normalized to UTC; the original offset is intentionally not preserved. Unparseable values → NULL |
| Array(inner) | ListArray | variable-length, child built recursively via `build_array` |
| Object(fields) | StructArray | nested Struct, children built recursively. Reached only via the synthesized storage tree's intermediates (Wave C transposes flat dotted-name leaves back into a nested Object before Arrow encoding). `Array(Object{...})` is rejected on the disk surface (TODO-0155), so no on-disk type produces this arm directly. |

**Per-row validity** follows the data: a file with `calibration: null` (or no `calibration` key) sees the `calibration` Struct column's validity bit set to 0 for that row, propagating to all descendant columns. A file with `calibration: {baseline: {intensity: 0.5}}` but no `wavelength` leaf sees the leaf's validity bit set to 0 while the intermediate Structs are valid.

**String preprocessing**: a `String` field is strict by default — non-string JSON values violate validation and never reach the storage layer. Fields declaring `preprocess = ["coerce-to-string"]` (often auto-inferred when mixed types were observed) accept any JSON value; non-strings are serialized to their JSON string representation before validation, then stored as strings. This preserves the "never silently drop data" contract for fields that opt in.

## Content Hash

`content_hash()` in `index/storage.rs`:

```rust
pub fn content_hash(content: &str) -> String {
    format!("{:016x}", xxh3_64(content.as_bytes()))
}
```

- Input: markdown body only (after frontmatter extraction by `gray_matter`)
- Algorithm: xxHash3-64
- Output: 16-character hex string

Frontmatter-only changes (editing a `status` field) do NOT trigger re-embedding. The hash covers the body that gets chunked and embedded. The `data` column is still rewritten on every chunk row at build time so frontmatter edits are reflected even when no embedding work happens.

## Build Metadata

`BuildMetadata` in `index/storage.rs` stores the build configuration snapshot:

| Key | Source |
|---|---|
| `mdvs.provider` | `EmbeddingModelConfig.provider` |
| `mdvs.model` | `EmbeddingModelConfig.name` |
| `mdvs.revision` | `EmbeddingModelConfig.revision` |
| `mdvs.chunk_size` | `ChunkingConfig.max_chunk_size` |
| `mdvs.glob` | `ScanConfig.glob` |
| `mdvs.built_at` | ISO 8601 timestamp |
| `mdvs.schema_hash` | xxh3-64 hex of `dsl_to_canonical(config)` serialized as canonical JSON |

Stored as table-level key-value metadata on the Lance dataset, written via `LanceBackend::write_index()` (the keys flow through the Arrow `Schema::metadata` map handed to `create_table`) and read via `LanceBackend::read_metadata()`.

**Schema hash** detects field-level changes (types, constraints, path-scoping, preprocessors) that don't show up in any of the other keys. Computed via `compute_schema_hash(config)` in `index/storage.rs`. Hashing the post-translation canonical JSON makes it whitespace-insensitive and key-order-insensitive. Pre-Wave-B datasets without this key read as `""` → treated as changed (conservative, requires `--force`).

**Config change detection**: build compares current config against stored `BuildMetadata` using `PartialEq`. Mismatch → requires `--force` for full rebuild. The schema-hash mismatch error reads: `"schema: fields, types, constraints, path-scoping, or preprocessors have changed"`. Search compares model identity → hard error on mismatch.

## Incremental Build

### Classification

`FileIndexEntry` in `index/storage.rs` is a lightweight projected read (only the columns needed to classify; the expensive `data` Struct + `embedding` columns are not fetched):

```rust
pub struct FileIndexEntry {
    pub file_id: String,
    pub filename: String,
    pub content_hash: String,
}
```

Classification in `cmd/build.rs` compares scanned files against the index:

| Classification | Condition | Action |
|---|---|---|
| **New** | filename not in index | Generate new file_id, chunk, embed |
| **Edited** | filename in index, hash differs | Keep file_id, re-chunk, re-embed |
| **Unchanged** | filename in index, hash matches | Skip chunking/embedding, retain existing chunks |
| **Removed** | in index, not in scan | Drop from output |

### Merge Strategy

1. Read retained chunk rows from the existing Lance dataset, projected to the columns we need; filter to `file_id`s of unchanged files.
2. Chunk and embed new + edited files.
3. Combine retained chunks with new chunks into a single Arrow `RecordBatch`.
4. Write the dataset from scratch via `Connection::create_table(...).mode(CreateTableMode::Overwrite)` — Lance handles the atomic replacement.
5. Rebuild the FTS (and, above 10k chunks, the vector) index inside the new dataset.

Model loading is skipped entirely when `needs_embedding == 0` (all files unchanged).

## Backend Abstraction

`Backend` enum at `index/backend.rs` has a single variant: `Backend::Lance(LanceBackend)`. The enum is kept (rather than collapsing to a struct) for forward compatibility with future remote-backend work and to keep the existing `LanceBackend::method()` call sites stable.

`LanceBackend` derives paths from root:
- `.mdvs/` — index directory
- `.mdvs/index.lance/` — Lance dataset (the table is named `index`)

Key methods: `write_index()` (builds the Arrow batch, calls `create_table(...).mode(Overwrite)`, then `build_indexes()`), `read_metadata()` (parses `BuildMetadata` from the Lance table-level kv), `read_file_index()` (lightweight projection for classification), `read_chunk_rows()` (full chunk rows for retained-file pass-through), `search()` (mode-dispatched LanceDB query + best-chunk-per-file dedupe), `index_stats()`, `exists()`.
