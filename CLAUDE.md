# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mdvs (Markdown Validation & Search) is a Rust CLI that treats markdown directories as databases — schema inference, frontmatter validation, and semantic search with SQL filtering. Single binary, no external services, instant embeddings via Model2Vec static models, DataFusion + Parquet for storage and search. Design specs live in `docs/spec/`.

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
- **`src/schema/`** — `shared.rs` (common types), `config.rs` (mdvs.toml)
- **`src/index/`** — `chunk.rs` (semantic chunking), `embed.rs` (model2vec embeddings), `storage.rs` (Parquet I/O)
- **`src/search.rs`** — cosine distance + DataFusion query
- **`src/cmd/`** — `init`, `build`, `search`, `check`, `update`, `clean`, `info`

### Data Pipeline

`.md` files → frontmatter extraction (`gray_matter`) → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → Parquet storage (`files.parquet` + `chunks.parquet`) → brute-force cosine distance in Rust → DataFusion SQL for JOIN/aggregate/filter

### Key Design Decisions

- Two layers: validation (init/update/check — no model needed) and search (build/search — model + parquets)
- Config-driven frontmatter fields: all frontmatter stored as native Arrow Struct column (`data`), no dynamic SQL columns. No interactive prompts.
- No lock file — `mdvs.toml` is the complete source of truth for validation. Build metadata stored in parquet native key-value metadata.
- Build includes check internally — validates before embedding, aborts on violations
- Model identity tracking in parquet metadata: hard error on model/revision mismatch for both search and build
- Note-level ranking uses max chunk similarity across chunks (not average)
- `--where` SQL clauses for metadata filtering (no custom filter syntax)
- `--output` global flag (`human`/`json`) via `CommandOutput` trait
- All text processing and vector math in Rust; DataFusion handles SQL query execution

### Storage

- Two artifacts: `mdvs.toml` (committed) + `.mdvs/` (gitignored)
- `files.parquet`: file_id, filename, frontmatter as `data` Struct column, content_hash, built_at
- `chunks.parquet`: chunk_id, file_id FK, chunk_index, start_line, end_line, embedding FixedSizeList<Float32>
- Build metadata (model, revision, chunk_size, glob, built_at) stored as parquet native key-value metadata

### Configuration

`mdvs.toml` sections: `[scan]`, `[embedding_model]`, `[chunking]`, `[update]`, `[search]`, `[fields]` + `[[fields.field]]`

- Validation sections (`[scan]`, `[update]`, `[fields]`): always present
- Build sections (`[embedding_model]`, `[chunking]`, `[search]`): added by `init --auto-build` or by `build`

### Commands

- `init [path]` — scan, infer schema, write `mdvs.toml`, optionally build
- `check [path]` — validate frontmatter against schema (read-only)
- `update [path]` — re-scan, infer new fields, update `mdvs.toml`
- `build [path]` — check + embed + write Parquets to `.mdvs/`
- `search <query> [path]` — query the index
- `info [path]` — show config and index status
- `clean [path]` — delete `.mdvs/` (deferred)

See `docs/spec/commands/` for detailed specs.

## Key Dependencies

| Crate | Purpose |
|---|---|
| `datafusion` | SQL query engine on Arrow arrays |
| `parquet` / `arrow` | Columnar storage and in-memory format |
| `model2vec-rs` | Static embedding inference (POTION models, no GPU) |
| `gray_matter` | YAML frontmatter extraction |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |
| `tokio` | Async runtime (required by DataFusion) |
