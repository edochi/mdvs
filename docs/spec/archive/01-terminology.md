# Terminology

**Status: DRAFT**

Canonical definitions for all terms used across mdvs specs. Every spec references this document rather than redefining terms.

---

## Core Concepts

### Vault

The target directory of markdown files that mdvs indexes. Any directory containing `.md` files — an Obsidian vault, a Hugo content directory, a Zettelkasten folder, a flat notes directory. mdvs makes no assumptions about the directory's structure or tooling.

### Frontmatter

YAML, TOML, or JSON metadata block at the top of a markdown file, delimited by `---` (YAML) or `+++` (TOML). Extracted by `gray_matter`. Not all files have frontmatter; files without it are still indexed with NULL metadata.

### Field

A single key in a file's frontmatter (e.g., `title`, `tags`, `date`). Fields have inferred or explicitly declared types. Defined in `mfv.toml` / `mdvs.toml`.

### Field Schema

The set of all field definitions in `mfv.toml` / `mdvs.toml`, including types and validation rules (`allowed`/`required` glob patterns, `pattern`, `values`). Shared between `mfv` and `mdvs`.

### Chunk

A segment of a markdown file's body, produced by semantic splitting via `text-splitter`'s `MarkdownSplitter`. Each chunk is a self-contained piece of content that respects a configurable maximum size. Chunks are the unit of embedding and vector search — search results resolve to chunks, which map back to files via `filename`.

### Plain Text

The result of stripping all markdown syntax from a chunk via `pulldown-cmark`. Only `Event::Text(...)` content is retained. This clean text is what gets embedded and stored in the `plain_text` column. The stripping removes bold, italic, links, code fences, headings markers, list markers, etc.

### Embedding

A fixed-size floating-point vector (`Vec<f32>`) representing the semantic meaning of a chunk's plain text. Produced by a static embedding model. Stored as `FixedSizeList<Float32>` in the chunks Parquet file where the list size equals the model's output dimension (e.g., 256).

### Content Hash

A hash (xxhash or blake3) of the full file content (frontmatter + body). Stored per-file in `mdvs.lock` `[[file]]` entries. Used for incremental builds — only files whose hash has changed are reprocessed.

---

## Models and Embeddings

### Static Embedding Model

A model that embeds text by tokenizing, looking up pre-computed token vectors in a matrix, and mean-pooling. No transformer forward pass, no GPU required, no context window. Inference is O(tokens) lookups. mdvs supports two formats — Model2Vec and Sentence Transformers StaticEmbedding — via a universal loader. See [Workflow: Model Loading](30-workflows/model-loading.md).

### Model2Vec

The static embedding format used by MinishLab's POTION models. Files: `embeddings.safetensors` (tensor key `"embeddings"`), `tokenizer.json`, `config.json`.

### Sentence Transformers StaticEmbedding

The static embedding format used by Sentence Transformers. Files: `model.safetensors` (tensor key `"embedding.weight"`), `tokenizer.json`, plus ST pipeline configs (ignored by mdvs). Some ST models support Matryoshka truncation.

### POTION Model

A specific family of Model2Vec models from `minishlab`. Available in various sizes: `potion-base-2M` (64-dim), `potion-base-32M`, `potion-retrieval-32M` (512-dim), `potion-multilingual-128M`.

### Matryoshka Truncation

A training technique (Matryoshka Representation Learning) where embeddings can be truncated to smaller dimensions with minimal quality loss. For example, a 1024-dim model truncated to 256-dim. Supported by some ST StaticEmbedding models. Configured via `truncate_dim` in `mdvs.toml`.

### Model Identity

Three values that uniquely identify the model used to produce embeddings in an artifact:

| Field | Source | Purpose |
|---|---|---|
| **Model ID** | HuggingFace repo ID (e.g., `minishlab/potion-multilingual-128M`) | Identifies the model family |
| **Model Dimension** | Output vector size (e.g., 256) | Schema validation for embedding column |
| **Model Revision** | Git commit SHA of the downloaded snapshot | Detects silent model weight updates |

Stored in `mdvs.lock` `[build]` section. See [Model Mismatch Workflow](30-workflows/model-mismatch.md).

---

## Storage

### Artifact

The `.mdvs/` directory at the root of the vault, containing compressed Parquet files produced by `mdvs build`. The searchable index. Analogous to `target/` in Cargo. Should be `.gitignore`-d.

### `files.parquet`

One row per markdown file. Contains the filename (primary key), dynamic field columns from the schema, a JSON metadata column for fields not in the schema, and a content hash. See [Storage Schema](20-storage/schema.md).

### `chunks.parquet`

One row per semantic chunk. Contains chunk ID, parent filename, chunk index, nearest heading, plain text, embedding vector, and character count.

### DataFusion

Pure Rust SQL query engine operating on Apache Arrow columnar data. Replaces DuckDB. Registers Parquet files as tables and executes SQL queries (JOIN, GROUP BY, WHERE) over them. Used for metadata filtering and note-level ranking during search.

### Parquet

Apache columnar file format for persistence. Supports compression (snappy/zstd), efficient column reads, and schema evolution. Used as the storage format in `.mdvs/`.

---

## Configuration

### `mfv.toml` / `mdvs.toml`

Field schema file shared between `mfv` and `mdvs`. Defines field types and validation rules (`allowed`/`required` glob patterns). Generated by `mfv init` or `mdvs init`. `mdvs.toml` additionally contains search-specific sections (`[model]`, `[chunking]`, `[behavior]`, `[search]`). See [Configuration](40-configuration/frontmatter-toml.md) and [Configuration: mdvs.toml](40-configuration/mdvs-toml.md).

### Lock File (`mfv.lock` / `mdvs.lock`)

Auto-generated snapshot of the resolved state (like `Cargo.lock`). `mfv.lock` contains field observations (which fields exist in which files). `mdvs.lock` is a superset: same field observations plus `[[file]]` entries with content hashes for staleness detection and a `[build]` section with artifact metadata (model identity, timestamps).

---

## Tools

### mdvs

The full semantic search CLI binary. Superset of mfv — does everything mfv does plus frontmatter content querying and vector search. Depends on `mdvs-schema` and `mfv` crates. Commands: `init`, `build`, `search`, `update`, `check`, `clean`, `info`.

### mfv (Markdown Frontmatter Validator)

Standalone frontmatter validation CLI binary (~2MB). No embeddings, no search. Independently publishable. Useful for CI pipelines, blog linting, documentation validation. Commands: `init`, `update`, `check`.

### `mdvs-schema`

Shared library crate. Defines field types, the type system, TOML parsing, field discovery, tree inference, and lock file types. Dependency of both `mfv` and `mdvs`.

---

## Operations

### Build

The process of creating or updating the `.mdvs/` artifact from markdown files. Scans files, extracts frontmatter, chunks content, computes embeddings, writes Parquet files. Incremental by default (only reprocesses changed files). `mdvs build --full` for clean rebuild. Analogous to `cargo build`.

### Incremental Build

The default build mode. Compares content hashes in `mdvs.lock` against the filesystem to determine which files are new, modified, deleted, or unchanged. Only reprocesses changed files.

### Staleness

Whether the artifact (`.mdvs/`) is up to date with the files on disk. Configured via `on_stale` in `mdvs.toml`: `"auto"` (default) transparently builds before search, `"strict"` errors if stale. CLI overrides: `--build` / `--no-build` on `mdvs search`.

### Note-Level Ranking

Search ranking strategy that groups chunk-level results by file. A file's score is the **maximum similarity** (minimum cosine distance) across all its chunks. The snippet and heading shown are from the best-matching chunk.

---

## Related Documents

- [Storage Schema](20-storage/schema.md)
- [Configuration](40-configuration/frontmatter-toml.md)
- [Configuration: mdvs.toml](40-configuration/mdvs-toml.md)
- [Workflow: Model Loading](30-workflows/model-loading.md)
- [Workflow: Model Mismatch](30-workflows/model-mismatch.md)
- [Crate: mdvs-schema](10-crates/mdvs-schema/spec.md)
- [Crate: mfv](10-crates/mfv/spec.md)
- [Crate: mdvs](10-crates/mdvs/spec.md)
