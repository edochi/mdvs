# mdvs — Markdown Directory Vector Search

Semantic search over directories of markdown files. Single binary, no external services, instant embeddings.

## What it does

`mdvs` indexes a directory of markdown files and provides semantic search using lightweight static embeddings. No GPU, no server, no API keys. It also validates frontmatter against a configurable schema.

## Architecture

Single Rust binary. Two layers:

1. **Validation layer** — frontmatter schema enforcement. Needs only `mdvs.toml` + files. No model.
2. **Search layer** — semantic search. Needs model + Parquet index on top.

### Data pipeline

```
.md files → frontmatter extraction (gray_matter)
          → semantic chunking (text-splitter MarkdownSplitter)
          → plain text extraction (pulldown-cmark)
          → embeddings (model2vec-rs)
          → Parquet storage (files.parquet + chunks.parquet)
          → cosine distance in Rust + DataFusion SQL
```

### Storage

| Artifact | Purpose | Git |
|----------|---------|-----|
| `mdvs.toml` | Config: scan, model, fields schema | committed |
| `.mdvs/` | Build artifacts: Parquet files with embedded metadata | gitignored |

## Commands

| Command | Layer | Description |
|---------|-------|-------------|
| `init` | Validation | Scan, infer schema, write `mdvs.toml` |
| `check` | Validation | Validate frontmatter against schema (read-only) |
| `update` | Validation | Re-scan, infer new fields, update `mdvs.toml` |
| `build` | Search | Check + embed + write Parquets |
| `search` | Search | Query the index |
| `info` | Utility | Show config and index status |

See `docs/spec/commands/` for detailed specs.

## Quick start

```bash
# Initialize (scan + infer + build index)
mdvs init ~/notes

# Search
mdvs search "how to handle errors in rust"

# Search with filters
mdvs search "async patterns" --where "tags = 'rust'" --limit 5

# Validate frontmatter
mdvs check

# Add new fields after adding files
mdvs update
```

## Key dependencies

| Crate | Purpose |
|-------|---------|
| `datafusion` | SQL query engine on Arrow arrays |
| `parquet` / `arrow` | Columnar storage and in-memory format |
| `model2vec-rs` | Static embedding inference (Model2Vec / POTION models) |
| `gray_matter` | YAML frontmatter extraction |
| `text-splitter` | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |

Default model: `minishlab/potion-base-8M`. Any Model2Vec-compatible model from HuggingFace works.
