# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mdvs (Markdown Directory Vector Search) is a Rust CLI for semantic search over directories of markdown files. Single binary, no external services, instant embeddings via Model2Vec static models, DataFusion + Parquet for storage and vector search. v0.1 (MVP) and v0.2 (workspace + mfv) are complete. Currently implementing v0.3 (usable mdvs CLI). README.md is the full design spec.

## Build Commands

```bash
cargo build                  # build all crates
cargo run -p mdvs            # run the search tool
cargo run -p mfv             # run the frontmatter validator
cargo test                   # run all tests
cargo test -p mdvs-schema    # run tests for a single crate
cargo clippy                 # lint
cargo fmt                    # format
```

## Architecture

Cargo workspace with three crates:

- **`crates/mdvs-schema/`** — library: field definitions, type system, TOML parsing. Shared by both binaries.
- **`crates/mfv/`** — library + binary (~2MB): standalone frontmatter validator. No embeddings, no storage. Independently publishable. Modules: `cmd/` (init, update, check, diff), `scan/` (extract, walk), `report/` (diagnostic, output, validate).
- **`crates/mdvs/`** — library + binary: full semantic search. Depends on both crates above. Modules: `cmd/` (init, build, search, check, update, clean, info), `storage/` (parquet, lock), `distance/` (cosine), `chunk.rs`, `embed.rs`.

### Data Pipeline

`.md` files → frontmatter extraction (`gray_matter`) → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → Parquet storage (`files.parquet` + `chunks.parquet`) → brute-force cosine distance in Rust → DataFusion SQL for JOIN/aggregate/filter

### Key Design Decisions

- Config-driven frontmatter fields: all frontmatter stored as JSON column, no dynamic SQL columns. No interactive prompts.
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

Both tools share the same TOML schema structure (`[[fields.field]]` array-of-tables format). Each tool looks for its own config file first:

- **`mfv.toml`** — standalone mfv users; `mfv check` precedence: `--schema` → `mfv.toml` → `mdvs.toml`
- **`mfv.lock`** — auto-generated discovery snapshot from `mfv init`. Captures all fields, types, counts.
- **`mdvs.toml`** — used by mdvs (also found by mfv as fallback). Contains field schema + search-specific sections (model, chunk size, storage, search defaults). Unknown sections silently ignored.

### mfv Commands

- `init` — discover fields, write config (`mfv.toml`) + lock (`mfv.lock`), print frequency table to stderr
  - `--dir <path>` (default `.`), `--glob <pattern>` (default `**`), `--config <path>` (default `mfv.toml`)
  - `--force` (overwrite existing), `--dry-run` (print table only), `--minimal`, `--include-bare-files`
  - `--frontmatter-format` (`both`/`yaml`/`toml`, default `both`)
- `update` — re-scan and refresh lock file (fails if validation doesn't pass)
- `check` — validate files against schema, exit 0 (valid) / 1 (errors) / 2 (runtime error)
- `diff` — compare current state against lock file, `--ignore-validation-errors`

### mdvs Commands

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
