# mdvs — Markdown Validation & Search

[![CI](https://github.com/edochi/mdvs/actions/workflows/ci.yml/badge.svg)](https://github.com/edochi/mdvs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Docs](https://img.shields.io/badge/docs-mdBook-green.svg)](https://edochi.github.io/mdvs/)

<div align="center">

  :x: A Document Database

  :white_check_mark: A Database for Documents

</div>

mdvs infers a schema from your frontmatter, validates it, and gives you semantic search with SQL filtering. Single binary, no cloud, no setup.

## Install

### Prebuilt binary (macOS / Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/edochi/mdvs/releases/latest/download/mdvs-installer.sh | sh
```

### From crates.io

```bash
cargo install mdvs
```

### From source

```bash
git clone https://github.com/edochi/mdvs.git
cd mdvs
cargo install --path .
```

## Quick Start

```bash
# Initialize: scans your files, infers a schema, builds a search index
mdvs init ~/notes

# Search with natural language
mdvs search "how to handle errors in rust"

# Filter results with SQL on frontmatter fields
mdvs search "async patterns" --where "draft = false" --limit 5

# Validate frontmatter against the inferred schema
mdvs check
```

That's it. No config files to write, no models to download manually, no services to start.

## Features

### Schema inference

mdvs scans your markdown files and infers a typed schema from frontmatter — field names, types (boolean, integer, float, string, arrays, nested objects), which directories they appear in, and which ones are required. The schema is written to `mdvs.toml` and can be customized.

```bash
mdvs init ~/notes
# Discovered 10 fields across 496 files
#   tags       String[]  (required in ["**"])
#   draft      Boolean   (allowed in ["blog/**"])
#   year       Integer   (required in ["articles/**"])
#   ...
```

### Frontmatter validation

Check your files against the schema — catch missing required fields, wrong types, and fields that appear where they shouldn't.

```bash
mdvs check
# blog/draft.md: missing required field 'tags'
# blog/old-post.md: field 'year' expected Integer, got String
```

### Semantic search

Instant vector search using lightweight static embeddings ([Model2Vec](https://github.com/MinishLab/model2vec)). The default model is 8MB — no GPU, no API keys, no network access needed at query time.

```bash
mdvs search "distributed consensus algorithms"
0.72  notes/raft.md
0.68  notes/paxos.md
0.61  blog/distributed-systems.md
```

All commands support `--output json` for scripting and pipelines:

```bash
mdvs search "distributed consensus" --output json
```

```json
{
  "hits": [
    { "filename": "notes/raft.md", "score": 0.72 },
    { "filename": "notes/paxos.md", "score": 0.68 },
    { "filename": "blog/distributed-systems.md", "score": 0.61 }
  ]
}
```

### SQL filtering

Filter search results on any frontmatter field using SQL syntax, powered by [DataFusion](https://datafusion.apache.org/).

```bash
mdvs search "rust" --where "draft = false AND year >= 2024"
mdvs search "recipes" --where "tags IS NOT NULL" --limit 5
```

### Incremental builds

Only changed files are re-embedded. Unchanged files keep their existing chunks and embeddings. If nothing changed, the model isn't even loaded.

```bash
mdvs build
# Built index: 3 new, 1 edited, 492 unchanged, 0 removed (4 files embedded)
```

## Commands

| Command | Description |
|---------|-------------|
| `init`  | Scan files, infer schema, write `mdvs.toml`, optionally build index |
| `check` | Validate frontmatter against schema |
| `update` | Re-scan and update field definitions |
| `build` | Validate + embed + write search index |
| `search` | Semantic search with optional SQL filtering |
| `info`  | Show config and index status |
| `clean` | Delete search index |

## How it works

mdvs treats your markdown directory like a database:

- **`init`** scans your files and infers a schema from frontmatter — like `CREATE TABLE`
- **`check`** validates every file against that schema — like constraint checking
- **`update`** detects new fields as your files evolve — like `ALTER TABLE`
- **`build`** chunks and embeds your content into a local Parquet index
- **`search`** queries that index with SQL filtering on metadata — like `SELECT ... WHERE ... ORDER BY similarity`

Two artifacts: `mdvs.toml` (committed, your schema) and `.mdvs/` (gitignored, the search index).

## Documentation

Full documentation at [edochi.github.io/mdvs](https://edochi.github.io/mdvs/).

## License

[MIT](LICENSE)
