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

### Metadata Column

The JSON column on the `mdfiles` table. Stores all frontmatter fields as a JSON object. Queryable via DataFusion JSON functions.

### Chunk

A segment of a markdown file's body, produced by semantic splitting via `text-splitter`'s `MarkdownSplitter`. Each chunk is a self-contained piece of content that respects a configurable maximum size. Chunks are the unit of embedding and vector search — search results resolve to chunks, which map back to files via `filename`.

### Plain Text

The result of stripping all markdown syntax from a chunk via `pulldown-cmark`. Only `Event::Text(...)` content is retained. This clean text is what gets embedded and stored in the `plain_text` column. The stripping removes bold, italic, links, code fences, headings markers, list markers, etc.

### Embedding

A fixed-size floating-point vector (`Vec<f32>`) representing the semantic meaning of a chunk's plain text. Produced by a Model2Vec model. Stored as `FLOAT[N]` in DuckDB where N is the model's output dimension (e.g., 256).

### Content Hash

A hash (xxhash or blake3) of the full file content (frontmatter + body). Stored per-file in `mdfiles.content_hash`. Used for incremental indexing — only files whose hash has changed are reprocessed.

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

A training technique (Matryoshka Representation Learning) where embeddings can be truncated to smaller dimensions with minimal quality loss. For example, a 1024-dim model truncated to 256-dim. Supported by some ST StaticEmbedding models. Configured via `truncate_dim` in `.mdvs.toml`.

### Model Identity

Three values that uniquely identify the model used to produce embeddings in a database:

| Field | Source | Purpose |
|---|---|---|
| **Model ID** | HuggingFace repo ID (e.g., `minishlab/potion-multilingual-128M`) | Identifies the model family |
| **Model Dimension** | Output vector size (e.g., 256) | Schema validation for `FLOAT[N]` |
| **Model Revision** | Git commit SHA of the downloaded snapshot | Detects silent model weight updates |

Stored in `vault_meta`. See [Model Mismatch Workflow](30-workflows/model-mismatch.md).

### HNSW Index

Hierarchical Navigable Small World graph index on the `chunks.embedding` column. Created by DuckDB's `vss` community extension. Enables fast approximate nearest neighbor search with cosine distance metric.

---

## Database

### Database File

The file `.mdvs.duckdb` located at the root of the vault. Co-located with the data so it's portable — move the directory, the index follows. Should be `.gitignore`-d.

### `vault_meta` Table

Key-value table storing index configuration: model identity, chunk size, vault path, timestamps. See [Database Schema](20-database/schema.md).

### `mdfiles` Table

One row per markdown file. Contains the filename (primary key), a JSON metadata column, and a content hash.

### `chunks` Table

One row per semantic chunk. Contains chunk ID, parent filename (FK to `mdfiles`), chunk index, nearest heading, plain text, embedding vector, and character count.

---

## Configuration

### `mfv.toml` / `mdvs.toml`

Field schema file shared between `mfv` and `mdvs`. Defines field types and validation rules (`allowed`/`required` glob patterns). Generated by `mfv init` or `mdvs init`. See [Configuration](40-configuration/frontmatter-toml.md).

### `.mdvs.toml`

Search-specific settings file used only by `mdvs`. Configures model selection, chunk sizing, storage options, search behavior, and search defaults. See [Configuration: .mdvs.toml](40-configuration/mdvs-toml.md).

---

## Tools

### mdvs

The full semantic search CLI binary (~20MB). Depends on `mdvs-schema` and `mfv` crates. Provides indexing, search, validation, and export commands.

### mfv (Markdown Frontmatter Validator)

Standalone frontmatter validation CLI binary (~2MB). No DuckDB, no embeddings. Independently publishable. Useful for CI pipelines, blog linting, documentation validation.

### `mdvs-schema`

Shared library crate. Defines field types, the type system, TOML parsing, field discovery, tree inference, and lock file types. Dependency of both `mfv` and `mdvs`.

---

## Operations

### Incremental Indexing

The default indexing mode. Compares content hashes to determine which files are new, modified, deleted, or unchanged. Only reprocesses changed files.

### Reindex

Full rebuild of all embeddings. Nulls all existing embeddings and recomputes them from stored `plain_text`. Does not require filesystem re-read. Triggered by `mdvs reindex`, typically after a model change.

### Note-Level Ranking

Search ranking strategy that groups chunk-level results by file. A file's score is the **maximum similarity** (minimum cosine distance) across all its chunks. The snippet and heading shown are from the best-matching chunk.

---

## Related Documents

- [Database Schema](20-database/schema.md)
- [Configuration](40-configuration/frontmatter-toml.md)
- [Configuration: .mdvs.toml](40-configuration/mdvs-toml.md)
- [Workflow: Model Loading](30-workflows/model-loading.md)
- [Workflow: Model Mismatch](30-workflows/model-mismatch.md)
- [Crate: mdvs-schema](10-crates/mdvs-schema/spec.md)
- [Crate: mfv](10-crates/mfv/spec.md)
- [Crate: mdvs](10-crates/mdvs/spec.md)
