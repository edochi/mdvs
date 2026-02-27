# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mdvs (Markdown Directory Vector Search) is a Rust CLI for semantic search over directories of markdown files. Single binary, no external services, instant embeddings via Model2Vec static models, DataFusion + Parquet for storage and vector search. v0.1 (MVP) and v0.2 (workspace + mfv) are complete. Currently implementing v0.3 (usable mdvs CLI). README.md is the full design spec.

## Build Commands

```bash
cargo build              # build
cargo run                # run mdvs
cargo test               # run all tests
cargo clippy             # lint
cargo fmt                # format
```

## Architecture

Single crate at the repo root. Modules grouped by pipeline stage:

- **`src/discover/`** — `scan.rs` (walk + parse YAML), `field_type.rs` (FieldType enum + widening), `infer.rs` (DirectoryTree + GlobMap + InferredSchema)
- **`src/schema/`** — `shared.rs` (common types), `config.rs` (mdvs.toml), `lock.rs` (mdvs.lock)
- **`src/index/`** — `chunk.rs` (semantic chunking), `embed.rs` (model2vec embeddings), `storage.rs` (Parquet I/O)
- **`src/search.rs`** — cosine distance + DataFusion query
- **`src/cmd/`** — `init`, `build`, `search`, `check`, `update`, `clean`, `info`

### Data Pipeline

`.md` files → frontmatter extraction (`gray_matter`) → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → Parquet storage (`files.parquet` + `chunks.parquet`) → brute-force cosine distance in Rust → DataFusion SQL for JOIN/aggregate/filter

### Key Design Decisions

- Config-driven frontmatter fields: all frontmatter stored as native Arrow Struct column, no dynamic SQL columns. No interactive prompts.
- Incremental indexing via content hashing in `mdvs.lock` (only re-process changed files)
- Model identity tracking in `mdvs.lock [build]`: hard error on model ID/dimension mismatch, warning on revision mismatch for search, hard error for build
- Note-level ranking uses max chunk similarity across chunks (not average)
- SQL WHERE clauses for metadata filtering (no custom filter syntax)
- All text processing and vector math in Rust; DataFusion handles SQL query execution

### Storage

- Directory: `.mdvs/` at root of target directory
- Two Parquet files: `files.parquet` (file_id UUID, filename, frontmatter JSON, content_hash, built_at), `chunks.parquet` (chunk_id UUID, file_id FK, chunk_index, start_line, end_line, embedding FixedSizeList<Float32>)
- Lock file: `mdvs.lock` (mirrors config + content hashes + build metadata)

### Configuration Files

- **`mdvs.toml`** — config (boundaries): field schema + search-specific sections (model, chunk size, storage, search defaults)
- **`mdvs.lock`** — lock (observed state): raw file lists per field, content hashes, build metadata

### Commands

- `init [path]` — discover fields, configure model, write `mdvs.toml` + `mdvs.lock`
- `build` — build or rebuild the search index in `.mdvs/`
- `search <query>` — search the index
- `check` — validate files against schema
- `update` — re-scan and refresh lock file
- `clean` — remove the `.mdvs/` directory
- `info` — show index info (model, file count, staleness)

## Key Dependencies

| Crate | Purpose |
|---|---|
| `datafusion` | SQL query engine on Arrow arrays |
| `parquet` / `arrow` | Columnar storage and in-memory format |
| `model2vec-rs` | Static embedding inference (POTION models, no GPU) |
| `gray_matter` | YAML/TOML/JSON frontmatter extraction |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |
| `tokio` | Async runtime (required by DataFusion) |

## Release Plan

v0.1 (single-file MVP) ✅ → v0.2 (workspace + mfv standalone) ✅ → **v0.3 (usable mdvs CLI)** → v0.4 (polish: similar, query, export) → v0.5 (integration: JSON output, MCP server). See README.md for detailed task lists per version.
