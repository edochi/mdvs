# Crate: `mdvs`

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Crate: mdvs-schema](../mdvs-schema/spec.md) | [Crate: mfv](../mfv/spec.md) | [Storage Schema](../../20-storage/schema.md)

---

## Overview

Full semantic search CLI binary. Superset of `mfv` — does everything `mfv` does (field discovery, schema validation) plus frontmatter content querying and vector search over file contents. Depends on `mdvs-schema` and `mfv`.

**Architecture:** DataFusion (pure Rust SQL on Arrow) for querying, compressed Parquet files in `.mdvs/` for persistence.

**Responsibilities:**

- Initialize vault: discover fields, infer schema, download model, write config + lock
- Build artifact: incremental ingestion, chunking, embedding, Parquet output
- Search: embed query, cosine distance, note-level ranking
- Model identity management and mismatch detection
- Delegate validation to `mfv` library
- Staleness management: auto-build before search

---

## CLI

```
mdvs <command> [options]

COMMANDS:
    init      Discover fields, set up model, create config + lock
    build     Scan files, extract, chunk, embed, write Parquet
    search    Semantic search across notes
    update    Re-scan and refresh lock file
    check     Validate frontmatter against schema (delegates to mfv)
    clean     Remove .mdvs/ artifact directory
    info      Show index status and statistics
```

### `mdvs init`

```
mdvs init [path] [--model <id>] [--glob <pattern>] [--config <path>]
                  [--force] [--dry-run] [--ignore-bare-files]
```

Subsumes `mfv init`. Discovers fields, infers types and allowed/required patterns via tree inference, writes config and lock, downloads the embedding model.

**Flags:**

| Flag | Default | Description |
|---|---|---|
| `[path]` | `.` | Directory to scan |
| `--model <id>` | `minishlab/potion-multilingual-128M` | HuggingFace model ID |
| `--glob <pattern>` | `**` | File matching glob (path scope, `.md` hardcoded) |
| `--config <path>` | `mdvs.toml` | Output config file path |
| `--force` | off | Overwrite existing config and lock |
| `--dry-run` | off | Print discovery table only, write nothing |
| `--ignore-bare-files` | off | Exclude files without frontmatter from inference |

**Steps:**

1. Scan directory, discover fields via `mdvs_schema::discover_fields`
2. Infer types and allowed/required patterns via `mdvs_schema::infer_field_paths`
3. Display frequency table to stderr
4. Write `mdvs.toml` (field schema + `[model]` section)
5. Write `mdvs.lock` (field observations + file hashes)
6. Download and cache embedding model via `model2vec-rs`

**If config already exists:** Error. User must `--force` to overwrite.

See [Workflow: Init](../../30-workflows/init.md).

### `mdvs build`

```
mdvs build [--full]
```

Scans the directory, processes files into the `.mdvs/` artifact. Incremental by default — only changed files are reprocessed. Implicitly refreshes the lock before building (like `cargo build` updates `Cargo.lock`).

**Flags:**

| Flag | Description |
|---|---|
| `--full` | Clean rebuild: remove `.mdvs/` and reprocess everything |

See [Workflow: Build](../../30-workflows/build.md).

### `mdvs search`

```
mdvs search <query> [--where <filter>] [-n <count>] [--format <fmt>]
                     [--chunks] [--build] [--no-build]
```

Embeds the query, computes cosine distance against all chunks, returns results ranked at the note level (or chunk level with `--chunks`).

**Flags:**

| Flag | Default | Description |
|---|---|---|
| `--where <filter>` | — | DataFusion SQL WHERE clause on files table |
| `-n <count>` | 10 | Number of results |
| `--format <fmt>` | `table` | Output format: `table`, `json`, `paths` |
| `--chunks` | off | Show chunk-level results instead of note-level grouping |
| `--build` | — | Force build before search (overrides `on_stale` config) |
| `--no-build` | — | Never auto-build (overrides `on_stale` config) |

**Auto-build behavior:**

| Config `on_stale` | `--build` | `--no-build` | Result |
|---|---|---|---|
| `auto` | — | — | Build if stale |
| `auto` | — | yes | Skip build |
| `strict` | — | — | Error if stale |
| `strict` | yes | — | Build if stale |
| any | yes | — | Always build |
| any | — | yes | Never build |

See [Workflow: Search](../../30-workflows/search.md).

### `mdvs update`

```
mdvs update [--dir <path>] [--config <path>]
```

Re-scans the directory and refreshes the lock file. Same as `mfv update` but for `mdvs.toml`/`mdvs.lock`. Does not modify config.

### `mdvs check`

```
mdvs check [--dir <path>] [--schema <path>] [--format <fmt>]
```

Delegates to `mfv::validate`. Validates all matching markdown files against the field schema. Convenience command so users don't need a separate `mfv` binary.

**Exit codes:** 0 = all valid, 1 = validation errors found, 2 = config/runtime error.

### `mdvs clean`

```
mdvs clean
```

Removes the `.mdvs/` artifact directory. Does not touch config or lock files.

### `mdvs info`

```
mdvs info
```

Displays: vault path, artifact size, file count, chunk count, model ID/dimension/revision, last build timestamp.

---

## Core Modules

### Ingestion

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
    /// Load a static embedding model, returning model + resolved identity.
    fn load_model(
        model_id: &str,
        revision: Option<&str>,
        truncate_dim: Option<usize>,
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

### Storage

```rust
mod storage {
    /// Build Arrow schema for files.parquet based on field definitions
    fn build_files_schema(fields: &[FieldDef]) -> Schema

    /// Write files and chunks to Parquet in .mdvs/
    fn write_artifact(
        dir: &Path,
        files: &RecordBatch,
        chunks: &RecordBatch,
    ) -> Result<()>

    /// Read Parquet files and register as DataFusion tables
    fn load_artifact(
        ctx: &SessionContext,
        dir: &Path,
    ) -> Result<()>

    /// Compute cosine distance and add as column to chunks RecordBatch
    fn add_distance_column(
        chunks: &RecordBatch,
        query_embedding: &[f32],
    ) -> Result<RecordBatch>
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

    enum Operation { Build, Search }

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

Configured via `on_stale` in `mdvs.toml` `[behavior]` section:

| Mode | Behavior |
|---|---|
| `auto` | Run incremental build before search if stale (default) |
| `strict` | Error if any files have changed since last build |

In `auto` mode, `mdvs search` transparently runs the equivalent of `mdvs build` first, so results are always fresh. The overhead is minimal for unchanged vaults (just a hash comparison).

CLI overrides: `--build` forces a build regardless of config, `--no-build` skips it.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `mdvs-schema` | Field definitions, type system, TOML parsing |
| `mfv` | Validation engine (library dependency) |
| `datafusion` | SQL query engine on Arrow |
| `parquet` | Parquet file I/O |
| `arrow` | Arrow columnar data types |
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
- [Storage Schema](../../20-storage/schema.md) — Parquet file schemas and type mappings
- [Workflow: Init](../../30-workflows/init.md) — full init flow
- [Workflow: Build](../../30-workflows/build.md) — incremental build pipeline
- [Workflow: Search](../../30-workflows/search.md) — query, rank, display
- [Workflow: Model Loading](../../30-workflows/model-loading.md) — format detection, universal loader
- [Workflow: Model Mismatch](../../30-workflows/model-mismatch.md) — identity checks
- [Configuration: Field Schema](../../40-configuration/frontmatter-toml.md)
- [Configuration: mdvs.toml](../../40-configuration/mdvs-toml.md)
