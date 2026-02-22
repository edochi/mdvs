# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

mdvs (Markdown Directory Vector Search) is a Rust CLI for semantic search over directories of markdown files. Single binary, no external services, instant embeddings via Model2Vec static models, DuckDB for storage and vector search. Currently in the design phase — README.md is the full design spec.

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

- **`crates/mdvs-schema/`** — library: field definitions, type system, TOML parsing for `frontmatter.toml`. Shared by both binaries.
- **`crates/mfv/`** — library + binary (~2MB): standalone frontmatter validator. No DuckDB, no embeddings. Independently publishable.
- **`crates/mdvs/`** — binary (~20MB): full semantic search. Depends on both crates above.

### Data Pipeline

`.md` files → frontmatter extraction (`gray_matter`) → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → DuckDB storage (`mdfiles` + `chunks` tables) → HNSW index (`vss` extension) → cosine distance search

### Key Design Decisions

- User-driven frontmatter field promotion: user chooses which fields become typed SQL columns during `init`, rest go to JSON metadata column
- Incremental indexing via content hashing (only re-process changed files)
- Model identity tracking in `vault_meta` table: hard error on model ID/dimension mismatch, warning on revision mismatch for search, hard error for index
- Note-level ranking uses max chunk similarity across chunks (not average)
- SQL WHERE clauses for metadata filtering (no custom filter syntax)
- All text processing in Rust; DuckDB handles only storage + vector search

### Database

- File: `.mdvs.duckdb` at root of target directory
- Three tables: `vault_meta` (config key-value), `mdfiles` (dynamic promoted columns + JSON metadata), `chunks` (text + FLOAT[N] embeddings)
- HNSW index on `chunks.embedding` with cosine metric via DuckDB `vss` extension

### Configuration Files

- **`frontmatter.toml`** — field schema shared between `mfv` and `mdvs` (types, validation rules, promoted flags)
- **`.mdvs.toml`** — search-specific settings (model, chunk size, storage, search defaults)

## Key Dependencies

| Crate | Purpose |
|---|---|
| `duckdb` (bundled) | Embedded database + vector search host |
| `model2vec-rs` | Static embedding inference (POTION models, no GPU) |
| `gray_matter` | YAML/TOML/JSON frontmatter extraction |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |

## Release Plan

v0.1 (single-file MVP) → v0.2 (workspace + mfv standalone) → v0.3 (usable mdvs CLI) → v0.4 (polish: similar, query, export) → v0.5 (integration: JSON output, MCP server). See README.md for detailed task lists per version.
