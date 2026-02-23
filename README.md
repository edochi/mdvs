# mdvs — Markdown Directory Vector Search

Semantic search over directories of markdown files. Single binary, no external services, instant embeddings.

## Motivation

Existing tools for semantic search over markdown notes either require external services (Ollama, OpenAI), heavy runtimes (Python, Node.js), or are tightly coupled to a specific editor. `mdvs` aims to be a standalone, zero-dependency CLI that indexes a directory of markdown files and provides semantic search using lightweight static embeddings — no GPU, no server, no API keys.

## Project Structure

The project is a Cargo workspace with three crates. The schema and validation layers are separated from the search tool so they can be used independently.

```
mdvs/
├── Cargo.toml                     # workspace root
├── crates/
│   ├── mdvs-schema/               # library: field definitions, type system, TOML parsing
│   │   └── src/
│   │       ├── lib.rs             # re-exports
│   │       ├── field_type.rs      # FieldType enum (String, Date, Boolean, etc.)
│   │       ├── field_def.rs       # FieldDef struct, RawFieldDef serde intermediate
│   │       ├── discovery.rs       # discover_fields(), auto_promote(), infer_type()
│   │       └── schema.rs         # Schema parsing, rules_for_path(), TOML roundtrip
│   ├── mfv/                       # library + binary: Markdown Frontmatter Validator
│   │   ├── src/
│   │   │   ├── lib.rs             # module re-exports
│   │   │   ├── main.rs            # CLI: scan + check subcommands
│   │   │   ├── extract.rs         # YAML/TOML frontmatter extraction → JSON
│   │   │   ├── scan.rs            # directory walking + glob filtering
│   │   │   ├── validate.rs        # validate(files, schema) → Vec<Diagnostic>
│   │   │   ├── diagnostic.rs      # Diagnostic + DiagnosticKind types
│   │   │   └── output.rs          # human / json / github formatters
│   │   └── tests/                 # integration tests (assert_cmd)
│   └── mdvs/                      # binary: full semantic search tool
│       └── src/main.rs            # depends on mdvs-schema + mfv
```

| Crate | Binary | Size | Use case |
|---|---|---|---|
| `mdvs-schema` | (library only) | — | Shared types: field definitions, type inference, schema TOML parsing. Embeddable in other tools. |
| `mfv` | `mfv` | ~2MB | Standalone frontmatter linter/validator. No DuckDB, no embeddings. |
| `mdvs` | `mdvs` | ~20MB | Full semantic search + validation. Depends on both crates above. |

Both tools share the same TOML schema structure but look for their own config file first. Standalone `mfv` users create `mfv.toml`; users of both tools share `mdvs.toml` — `mfv` discovers it via fallback. The schema format (field definitions, types, validation rules) is identical in both files. Unknown sections are silently ignored, so `mfv` works fine with an `mdvs.toml` that contains search-specific settings.

## Tech Stack

### Core Dependencies (Rust)

| Crate | Purpose |
|---|---|
| `duckdb` (bundled) | Embedded database, SQL engine, vector search host |
| `model2vec-rs` | Static embedding inference (Model2Vec / POTION models) |
| `gray_matter` | YAML/TOML/JSON frontmatter extraction from markdown files |
| `text-splitter` (with `markdown` feature) | Semantic chunking of markdown — splits on headings, paragraphs, sentences as needed to respect size limits |
| `pulldown-cmark` | CommonMark pull parser — plain text extraction from markdown chunks (stripping syntax for embeddings) |
| `clap` | CLI argument parsing |
| `anyhow` | Error handling |
| `xxhash-rust` or `blake3` | Content hashing for incremental indexing |
| `walkdir` | Filesystem traversal |
| `indicatif` | Progress bars for init/indexing |

All Rust crates compile statically into the binary. No runtime downloads except the DuckDB vss extension and the embedding model on first run.

### DuckDB Community Extensions (loaded at runtime via SQL)

| Extension | Purpose |
|---|---|
| `vss` | HNSW index on embedding columns, `array_cosine_distance()` |

Previous iterations considered using DuckDB's `yaml` and `markdown` community extensions for frontmatter parsing and section splitting. These were replaced with native Rust crates (`gray_matter`, `text-splitter`, and `pulldown-cmark`) for three reasons: eliminates two runtime extension downloads, gives finer control over parsing and chunking behavior, and keeps the Rust → DuckDB boundary clean (Rust handles all text processing, DuckDB handles storage + vector search).

### Embedding Models

Default model: `minishlab/potion-multilingual-128M` (256-dimensional, ~30MB, 101 languages).

Any Model2Vec-compatible model from HuggingFace works — the user provides a model identifier string, and `model2vec-rs` handles download, caching, and inference. Model2Vec models are static embedding models: they tokenize, look up pre-computed token embeddings, and mean-pool. No transformer forward pass, no GPU needed. Inference is effectively instant.

Available models in the POTION family:

| Model | Params | Dimensions | Use case |
|---|---|---|---|
| `minishlab/potion-base-2M` | 1.8M | 256 | Smallest, English |
| `minishlab/potion-base-4M` | 3.7M | 256 | Small, English |
| `minishlab/potion-base-8M` | 7.5M | 256 | Good balance, English |
| `minishlab/potion-base-32M` | 32.3M | 256 | Best English |
| `minishlab/potion-retrieval-32M` | 32.3M | 256 | Optimized for retrieval |
| `minishlab/potion-multilingual-128M` | 128M | 256 | Multilingual, 101 languages |

## Architecture

```
  .md files on disk (with optional YAML frontmatter)
        │
        ▼  Rust: gray_matter extracts frontmatter + body
        │
        ▼  Rust: text-splitter MarkdownSplitter → semantic chunks (respecting size limit)
        │
        ▼  Rust: pulldown-cmark strips markdown syntax from each chunk → plain text
        │
        ├──→ DuckDB: mdfiles table (user-chosen promoted fields + JSON metadata, hash)
        │
        ├──→ DuckDB: chunks table (plain text per chunk)
        │
        ▼  Rust: model2vec-rs encodes plain_text per chunk → Vec<f32>
        │
  chunks table: ... + embedding FLOAT[N]
        │
        ▼  DuckDB: HNSW index on chunks.embedding (cosine metric, via vss extension)
        │
  Queryable:  chunks ──JOIN──→ mdfiles
              array_cosine_distance(embedding, query_vec)
              + WHERE filters on promoted fields and JSON metadata
```

### Data Flow

1. **Ingestion**: Rust reads each .md file, uses `gray_matter` to extract YAML frontmatter and separate the markdown body. User-chosen promoted fields (selected during `init`) become typed columns in the `mdfiles` table; everything else goes into a JSON metadata column. Notes without frontmatter get NULLs + empty JSON.
2. **Chunking**: Rust uses `text-splitter`'s `MarkdownSplitter` to split the markdown body into semantic chunks. The splitter respects a configurable maximum chunk size (in characters) and cascades through semantic levels — preferring splits at heading boundaries, then paragraphs, then sentences, then words. Each chunk is then run through `pulldown-cmark` to extract plain text (stripping all markdown syntax) for clean embedding input.
3. **Embedding**: Rust calls `model2vec-rs` to encode each chunk's plain text, writes the resulting vectors back to the `chunks` table.
4. **Indexing**: DuckDB's vss extension creates/rebuilds an HNSW index on the `chunks.embedding` column for fast approximate nearest neighbor search.
5. **Querying**: The user's search query is embedded with the same model in Rust, then DuckDB performs a cosine distance query against the HNSW index on chunks, joined back to mdfiles for metadata filtering and display.

### Database Location

The database file `.mdvs.duckdb` lives at the root of the target directory. It's co-located with the data, portable (move the directory, the index follows), and `.gitignore`-able.

### Schema

```sql
-- Metadata about the vault index itself
CREATE TABLE vault_meta (
    key   VARCHAR PRIMARY KEY,
    value VARCHAR
);
-- Stores:
--   model_id           HuggingFace repo ID (e.g. "minishlab/potion-multilingual-128M")
--   model_dimension    Output vector size (e.g. "256")
--   model_revision     Git commit SHA of the downloaded model snapshot
--   promoted_fields    JSON array of user-chosen promoted frontmatter fields (e.g. '["title","tags","date"]')
--   max_chunk_size     Maximum chunk size in characters (e.g. "1000")
--   vault_path         Absolute path to directory root at index time
--   glob_pattern       File glob used for indexing
--   created_at         When the index was first created
--   last_indexed_at    When the last index completed

-- One row per markdown file
-- Promoted columns are generated dynamically at init based on the user's
-- field schema (defined in mdvs.toml). No hardcoded columns.
-- Example below assumes the user promoted title, tags, date:
CREATE TABLE mdfiles (
    filename      VARCHAR PRIMARY KEY,  -- relative path from directory root
    -- promoted frontmatter fields (varies per directory, from mdvs.toml):
    title         VARCHAR,              -- NULL if absent in frontmatter
    tags          VARCHAR[],            -- NULL if absent in frontmatter
    date          DATE,                 -- NULL if absent in frontmatter
    -- end promoted fields
    metadata      JSON,                 -- all non-promoted frontmatter fields
    content_hash  VARCHAR,              -- xxhash/blake3 of full file content
    indexed_at    TIMESTAMP DEFAULT current_timestamp
    -- raw_content VARCHAR             -- optional, enabled via store_raw_content = true
);

-- One row per semantic chunk of a note
CREATE TABLE chunks (
    chunk_id      VARCHAR PRIMARY KEY,  -- e.g. "path/to/note.md#0", "path/to/note.md#1"
    filename      VARCHAR NOT NULL REFERENCES mdfiles(filename),
    chunk_index   INTEGER,              -- 0-based position within the note
    heading       VARCHAR,              -- nearest heading ancestor extracted from chunk content (NULL if none)
    plain_text    VARCHAR,              -- markdown-stripped text content of this chunk
    embedding     FLOAT[N],             -- N determined by model dimension
    char_count    INTEGER               -- character count (useful for debugging chunk sizing)
);

-- HNSW index for vector search on chunks
CREATE INDEX chunks_hnsw ON chunks USING HNSW (embedding)
    WITH (metric = 'cosine');
```

Note: `N` in `FLOAT[N]` is determined at init time based on the chosen model's output dimension (e.g., 256 for POTION models). The dimension is stored in `vault_meta` and validated on every run.

#### Frontmatter Handling

Not all notes have frontmatter, and those that do may have wildly different fields depending on the user's workflow (Obsidian, Zettelkasten, blog, research notes, etc.). Rather than hardcoding assumptions about which fields matter, `mdvs` discovers them from the config file.

**Field discovery** is done via `mfv scan`, which walks the directory, extracts frontmatter from all matching files, and produces a frequency table. The `--output` flag writes this as a config file. Fields are auto-promoted based on a configurable threshold (fraction of files a field must appear in). The user can then edit the config to adjust types, add validation rules, and toggle `promoted` flags.

Field definitions (including `promoted = true`) are stored in `mdvs.toml`, and the promoted list is also stored in `vault_meta` for runtime validation. No interactive prompts — everything is config-driven.

**Type inference** for promoted fields:

- If values are consistently YAML lists → `VARCHAR[]`
- If values parse as dates → `DATE`
- If values are booleans → `BOOLEAN`
- Everything else → `VARCHAR`

The user can override inferred types in config if needed, but the defaults should be right for 95% of cases.

**Non-promoted fields** are still captured in the `metadata` JSON column, fully queryable via DuckDB's JSON functions (e.g., `metadata->>'author'`, `json_extract(metadata, '$.custom_field')`).

**No frontmatter at all**: the note still gets indexed — promoted columns are NULL, metadata is `{}`, and the full content is still chunked and embedded.

**Config-driven**: field promotion is controlled entirely by the config file. There are no interactive prompts — `mfv scan --output` generates the initial config, and the user edits it from there.

#### Chunking Strategy

Notes are split into semantic chunks using `text-splitter`'s `MarkdownSplitter`. Unlike a simple heading-based split, it cascades through semantic levels to produce chunks that respect a configurable maximum size while splitting at the most meaningful boundary possible:

1. **Heading boundaries** (preferred) — sections stay intact if they fit within the size limit
2. **Block elements** — paragraphs, code blocks, list items
3. **Sentence boundaries** — via Unicode segmentation
4. **Word boundaries** — last resort before character-level

This handles all note shapes gracefully:

- **Short notes** (under the size limit): one chunk for the entire note.
- **Structured notes** with headings: splits along heading boundaries, keeping sections intact when possible.
- **Long prose notes** without headings: splits at paragraph/sentence boundaries instead of producing one enormous chunk.
- **Very long sections**: automatically sub-splits at paragraph or sentence level, which was previously deferred to v0.4.

The default `max_chunk_size` is 1000 characters, configurable in `mdvs.toml` or via `--chunk-size` at init. Character-based sizing is sufficient since model2vec handles variable-length input; token-precise splitting is unnecessary.

**Plain text extraction**: `text-splitter` returns chunks that still contain markdown formatting. Each chunk is then passed through `pulldown-cmark` to extract only `Event::Text(...)` content, stripping all syntax (bold, italic, links, code fences, etc.). This clean plain text is what gets embedded and stored in `plain_text`.

**Heading extraction**: since `text-splitter` doesn't return structured section metadata, the chunking pipeline does a lightweight pass over each chunk's markdown to find the first or most prominent heading. This is stored in the `heading` column for display in search results (the `§ Section Title` indicator). Chunks with no heading get NULL.

Search results return chunks, which resolve back to notes via the `filename` foreign key. This means a search can surface a specific chunk deep inside a long note, not just the note as a whole.

### Incremental Indexing Strategy

On each `mdvs index` run:

1. Walk the target directory, compute a content hash for each .md file.
2. Query existing hashes from the `mdfiles` table.
3. **New files**: insert into `mdfiles`, split into chunks, insert chunks + compute embeddings.
4. **Modified files** (hash changed): update `mdfiles` row, delete old chunks, re-split, insert new chunks + compute embeddings.
5. **Deleted files** (in DB but not on disk): delete from `mdfiles` (cascades to `chunks`).
6. **Unchanged files**: skip entirely.
7. Rebuild the HNSW index (fast for typical vault sizes).

Raw text (`plain_text`) is stored per-chunk in the database, so a model change only requires recomputing embeddings — no filesystem re-read or re-parsing needed.

### Model Identity and Mismatch Detection

Embeddings from different models (or different versions of the same model) are incompatible — mixing them in the same index produces meaningless search results. The database stores three identity fields in `vault_meta` to catch this:

- **`model_id`** — the HuggingFace repo ID (e.g., `minishlab/potion-multilingual-128M`). This is what the user configures and what appears in error messages.
- **`model_dimension`** — output vector size (e.g., 256). Used for schema validation (`FLOAT[N]`). A dimension mismatch would crash SQL queries, so this is always a hard error.
- **`model_revision`** — the Git commit SHA of the downloaded model snapshot. This catches silent model updates where the repo ID stays the same but the weights change.

The revision is resolved from the HuggingFace cache directory structure (`~/.cache/huggingface/hub/models--org--name/snapshots/<sha>/`). If `model2vec-rs` exposes the commit SHA directly, that's preferred. If the user pins a specific revision in config, that exact revision is downloaded and recorded.

#### Mismatch Logic

Every operation that touches embeddings (`index`, `search`, `similar`) checks model identity before proceeding:

1. **Dimension mismatch** → hard error, always. The FLOAT[N] column would reject the vectors anyway.
   ```
   Error: Dimension mismatch.
     Database schema expects:  FLOAT[256]  (model: minishlab/potion-multilingual-128M)
     Current model produces:   FLOAT[384]  (model: minishlab/some-other-model)

   Run `mdvs index --full` to rebuild with the new model.
   ```

2. **Model ID mismatch** → hard error. Different models produce incompatible embedding spaces.
   ```
   Error: Model mismatch.
     Database was indexed with: minishlab/potion-multilingual-128M
     Current config uses:       minishlab/potion-base-32M

   Embeddings are incompatible across different models.
   Options:
     • Switch back:  mdvs --model minishlab/potion-multilingual-128M search "query"
     • Reindex all:  mdvs index --full
   ```

3. **Revision mismatch** (same model ID, different commit SHA) → warning, not error. The vectors are probably close but not identical. Search still works, results may be slightly off.
   ```
   Warning: Model revision changed.
     Database indexed with revision: a1b2c3d
     Current model revision:         e4f5g6h

   Results may be slightly inconsistent. Run `mdvs index --full` for clean results.
   ```

4. **All match** → proceed normally.

For `index` specifically (which writes new embeddings), a revision mismatch is promoted to a hard error — we don't want to mix embeddings from different revisions in the same index. The user must either pin the old revision or run `index --full`.

#### Reindex on Model Change

`mdvs index --full` sets all embeddings to NULL and recomputes them. Because `plain_text` is stored per-chunk, this requires no filesystem re-read or re-parsing — just re-embedding. For a 5,000-note vault with static embeddings, this takes seconds.

No migration adapters, no dual indexes. Just full rebuild.

## CLI Design

```
mdvs <command> [options]

COMMANDS:
    init      Initialize a new index (create DB, download model, generate config)
    index     Build or update the index (incremental by default, --full for rebuild)
    search    Semantic search across notes
    similar   Find notes similar to a given note (v0.4)
    validate  Validate frontmatter against schema (delegates to mfv)
    info      Show index status and statistics
    query     Run raw SQL against the indexed data (future)
    export    Export database tables as Parquet files (future)
```

### Global Options

```
--db <path>       Path to .duckdb file (default: ./.mdvs.duckdb)
--dir <path>      Path to markdown directory root (default: .)
--model <id>      Override HuggingFace model ID (checked against DB)
--revision <sha>  Override model revision (checked against DB)
```

### `mdvs init`

```
mdvs init [--model <hf-model-id>] [--revision <commit-sha>] [--glob <pattern>] [--chunk-size <n>]

Options:
    --model        HuggingFace model ID (default: minishlab/potion-multilingual-128M)
    --revision     Pin a specific model revision by Git commit SHA (default: latest)
    --glob         File glob pattern (default: **/*.md)
    --chunk-size   Maximum chunk size in characters (default: 1000)
```

Creates the .duckdb file, installs/loads the DuckDB vss extension, creates the schema with promoted columns from `mdvs.toml` (generates a default config if missing), downloads and caches the embedding model. Stores all configuration in `vault_meta`. No interactive prompts — field promotion is driven entirely by the config file.

If `--revision` is omitted, the latest available revision is downloaded and its SHA is recorded. If specified, that exact revision is fetched from HuggingFace. Pinning a revision is recommended for reproducibility — it prevents silent model updates from causing revision mismatch warnings.

### `mdvs index`

```
mdvs index [--full]

Options:
    --full     Force full rebuild (replaces the old `reindex` command — updates vault_meta
               with new model identity, NULLs all embeddings, recomputes)
```

Scans the directory, diffs against stored hashes, processes only changed files. Default mode is incremental. Use `--full` after a model change to rebuild all embeddings (no filesystem re-read needed since `plain_text` is stored per-chunk).

### `mdvs search`

```
mdvs search <query> [--where <filter>] [-n <count>] [--format <fmt>] [--chunks]

Options:
    --where    SQL WHERE clause filter on mdfiles table (e.g. "tags @> ['rust']", "date > '2025-01-01'")
    -n         Number of results (default: 10)
    --format   Output format: table (default), json, paths
    --chunks   Show individual chunk results instead of grouping by note
```

Filters use SQL expressions directly against the promoted columns and metadata JSON. This avoids inventing a custom filter syntax — users get the full power of DuckDB's SQL, and it works with whatever promoted fields they chose at init.

Common filter examples (assuming title/tags/date are promoted):

```bash
mdvs search "crdt resolution" --where "tags @> ['rust']"
mdvs search "authentication" --where "date > '2025-01-01'"
mdvs search "deployment" --where "metadata->>'author' = 'edoardo'"
```

Example output:

```
── Results for "how does CRDT conflict resolution work" ──

 1. projects/collabide/crdt-design.md § Conflict Resolution    0.142
    [rust, crdt, collaborative]  2025-06-12
    Operational Transform vs CRDT approaches for the editor...

 2. reading/kleppmann-crdt-paper.md § Summary                  0.198
    [papers, distributed-systems]  2025-03-20
    Notes on Martin Kleppmann's paper on conflict-free...

2 results (8ms search, 1ms embed)
```

Results show the note filename and the best-matching chunk's heading (`§ Heading`), so you know approximately where in the note the match was found.

#### Note-Level Ranking

Search operates on chunks but displays results grouped by note. The ranking strategy:

- **Score**: the **maximum similarity** (minimum cosine distance) across all chunks of a note. This ensures a note with one highly relevant section ranks above a note that's vaguely related across many sections.
- **Snippet**: the plain text of the best-matching chunk (the one that produced the max similarity).
- **Heading**: the heading associated with that best-matching chunk (if any).

```sql
WITH ranked_chunks AS (
    SELECT
        c.filename,
        c.heading,
        LEFT(c.plain_text, 120) AS snippet,
        array_cosine_distance(c.embedding, ?::FLOAT[N]) AS distance
    FROM chunks c
)
SELECT
    m.filename,
    m.title,
    m.tags,
    m.date,
    MIN(rc.distance) AS distance,
    FIRST(rc.heading ORDER BY rc.distance) AS best_heading,
    FIRST(rc.snippet ORDER BY rc.distance) AS snippet
FROM ranked_chunks rc
JOIN mdfiles m ON rc.filename = m.filename
GROUP BY m.filename, m.title, m.tags, m.date
ORDER BY distance
LIMIT ?;
```

Note: the `m.title`, `m.tags`, `m.date` references in the query above are illustrative — the actual columns depend on what the user promoted at init. The query is generated dynamically based on `vault_meta.promoted_fields`.

A `--chunks` flag bypasses grouping and returns raw chunk-level results, useful for finding specific sections across different notes.

### `mdvs similar`

```
mdvs similar <file> [-n <count>]
```

Looks up the embedding for the given file, uses it as the query vector. No model inference needed.

### `mdvs query`

```
mdvs query <sql>
mdvs query -        # read SQL from stdin
```

Direct SQL access to the DuckDB database for ad-hoc queries.

### `mdvs export`

```
mdvs export [--output <dir>]

Options:
    --output   Output directory (default: ./mdvs-export/)
```

Exports the database tables as Parquet files:

```
mdvs-export/
├── mdfiles.parquet       # note metadata (no raw content, to keep it lean)
├── chunks.parquet      # chunk text + embeddings
└── vault_meta.parquet  # index metadata
```

DuckDB uses its own internal columnar format (not Parquet) for storage, but `COPY TO ... (FORMAT PARQUET)` is a first-class operation. Parquet export is useful for:

- Feeding into other tools (Polars, pandas, DuckDB CLI, Spark)
- Sharing the index without sharing the .duckdb binary format
- Archival / version control of the index state
- Loading into a different DuckDB instance for analysis

### `mdvs validate`

```
mdvs validate [--dir <path>] [--schema <path>]
```

Delegates to the `mfv` validation engine (the `mfv` crate is a library dependency). Validates all markdown files against the field schema defined in `mdvs.toml`. This is a convenience command — it runs the same logic as `mfv check` but from within the `mdvs` binary.

### `mdvs info`

```
mdvs info
```

Shows: directory path, DB size, file count, chunk count, model ID/dimension/revision, promoted fields, max chunk size, last indexed timestamp.

## mfv — Markdown Frontmatter Validator

`mfv` is a standalone CLI for validating markdown frontmatter against a schema. It ships as its own binary (~2MB) with no DuckDB, no embeddings, no search. Useful for bloggers, documentation maintainers, CI pipelines — anyone with markdown + frontmatter who wants linting without vector search.

### mfv CLI

```
mfv <command> [options]

COMMANDS:
    scan      Discover frontmatter fields, print frequency table, optionally write config
    check     Validate files against schema
```

### `mfv scan`

```
mfv scan [--dir <path>] [--glob <pattern>] [--threshold <f64>] [--output [path]]

Options:
    --dir         Directory to scan (default: .)
    --glob        File glob pattern (default: **/*.md)
    --threshold   Auto-promote threshold — fraction of files a field must appear in (default: 0.5)
    --output      Write config to file; flag with no value defaults to mfv.toml
```

Scans markdown files, discovers frontmatter fields via type inference, and prints a frequency table to stdout. Status messages go to stderr. Fields appearing in at least `--threshold` fraction of files are marked as promoted.

```
$ mfv scan

Field                Type          Count/Total Promoted
--------------------------------------------------------
title                string          11/12     Y
tags                 string[]        10/12     Y
date                 date             9/12     Y
draft                boolean          4/12
rating               float            1/12
```

With `--output`, the discovered schema is written as a TOML config file. This is the starting point for building a validation schema — the user edits the generated file to add `required`, `pattern`, and `values` rules.

### `mfv check`

```
mfv check [--dir <path>] [--schema <path>] [--format <fmt>]

Options:
    --dir       Directory to validate (default: .)
    --schema    Path to schema file (overrides auto-discovery)
    --format    Output format: human (default), json, github
```

Validates all matching files against the schema. Schema resolution precedence (when `--schema` is not given): `mfv.toml` in target dir → `mdvs.toml` in target dir → error.

Exit codes: 0 = all valid, 1 = validation errors found, 2 = config/runtime error.

```
$ mfv check

Checking 1203 files against mfv.toml

blog/half-finished-post.md
  - status: required field missing

papers/new-idea.md
  - doi: value "not-a-doi" does not match pattern /^10\.\d{4,9}/.*/

notes/quick-thought.md
  - tags: expected type 'string[]', got 'string'

3 error(s) in 3 file(s)
```

The `--format github` mode outputs GitHub Actions annotations (`::error file=...`) for CI integration. The `--format json` mode outputs a JSON array of `{file, field, message}` objects.

## Configuration

Both `mfv` and `mdvs` share the same TOML schema structure. The tools look for their own config file first:

- **`mfv`** reads: `--schema` flag → `mfv.toml` → `mdvs.toml` → error
- **`mdvs`** always reads/writes: `mdvs.toml`

Standalone `mfv` users get `mfv.toml` (generated by `mfv scan --output`). Users of both tools share `mdvs.toml` — `mfv` discovers it via fallback. Unknown TOML sections are silently ignored, so `mfv` works fine with an `mdvs.toml` that contains search-specific sections.

### Field Schema

```toml
glob = "**/*.md"

[fields]
promote_threshold = 0.5         # auto-promote fields appearing in ≥50% of files

[[fields.field]]
name = "title"
type = "string"
promoted = true                 # becomes a typed column in mdvs's mdfiles table

[[fields.field]]
name = "tags"
type = "string[]"
promoted = true

[[fields.field]]
name = "date"
type = "date"
promoted = true

[[fields.field]]
name = "status"
type = "enum"
values = ["draft", "review", "published"]
required = true
paths = ["blog/**"]             # only required in files matching this glob

[[fields.field]]
name = "doi"
type = "string"
required = true
paths = ["papers/**"]
pattern = "^10\\.\\d{4,9}/.*"  # regex validation
promoted = true
```

**Supported types**: `string`, `string[]`, `date`, `boolean`, `integer`, `float`, `enum`

**Validation rules** (all opt-in, all ignored if absent):

| Rule | Type | Meaning |
|---|---|---|
| `required` | `bool` | Field must be present (in all files, or scoped by `paths`) |
| `paths` | `string[]` | Glob patterns where the field's rules apply |
| `pattern` | `string` | Regex the value must match (strings only) |
| `values` | `string[]` | Allowed values (enum type) |
| `promoted` | `bool` | mdvs-specific: becomes a SQL column vs. staying in JSON metadata |

Type inference (when `type` is not explicitly set): YAML lists → `string[]`, parseable dates → `date`, booleans → `boolean`, numbers → `integer`/`float`, everything else → `string`.

Fields not listed in the config are auto-discovered, type-inferred, and promoted or not based on `promote_threshold`. A minimal config with just promoted fields and no validation rules is ~10 lines.

### Search Settings (mdvs only)

These sections are only used by `mdvs` and are ignored by `mfv`:

```toml
[model]
name = "minishlab/potion-multilingual-128M"
# revision = "a1b2c3d4e5f6"  # optional: pin to specific HF commit SHA

[chunking]
max_chunk_size = 1000  # characters

[storage]
store_raw_content = false       # if true, adds raw_content VARCHAR to mdfiles

[behavior]
on_stale = "auto"               # "auto" = incremental sync before search, "strict" = error if stale

[search]
default_limit = 10
snippet_length = 120
```

## Release Plan

### v0.1 — MVP (Proof of Concept) ✅

Goal: validate that all the pieces fit together. Single-crate prototype, not yet a workspace.

- [x] Single `main.rs`, no subcommands yet
- [x] Open DuckDB, load vss extension
- [x] Read .md files with `walkdir`, extract frontmatter via `gray_matter` → mdfiles table
- [x] Split markdown body with `text-splitter` MarkdownSplitter → semantic chunks
- [x] Extract plain text from each chunk via `pulldown-cmark` (strip markdown syntax)
- [x] Extract nearest heading from each chunk for display metadata
- [x] Load a Model2Vec model via `model2vec-rs`
- [x] Compute embeddings per chunk, insert into DuckDB as `FLOAT[256]`
- [x] Create HNSW index on chunks via vss extension
- [x] Hardcoded query, print results joined back to mdfiles
- [x] **Validated**: gray_matter handles real Obsidian frontmatter, text-splitter produces sensible chunks, pulldown-cmark plain text extraction is clean, duckdb-rs handles FLOAT[N] via SQL literals, vss extension loads cleanly, model2vec-rs loads potion model from HF cache, two-table join works with HNSW

### v0.2 — Workspace + mfv ✅

Goal: extract the schema and validation layers into their own crates. Ship `mfv` as a standalone tool.

- [x] Restructure into Cargo workspace: `mdvs-schema`, `mfv`, `mdvs`
- [x] `mdvs-schema`: field definition types, `[[fields.field]]` TOML parsing, type inference engine
- [x] `mfv scan`: discover frontmatter fields, print frequency table, auto-promote by threshold, optionally write config
- [x] `mfv check`: validate files against schema, human/json/github output formats
- [x] Schema resolution precedence: `--schema` → `mfv.toml` → `mdvs.toml` → error
- [x] Validation rules: `required`, `paths` (glob-scoped via globset), `pattern` (regex), `values` (enum)
- [x] Exit codes: 0 = valid, 1 = errors, 2 = config error
- [x] Comprehensive test suite: 39 unit tests + 22 integration tests (assert_cmd)

Deliverable: `mfv` is independently useful and publishable. Someone running a Hugo blog can `cargo install mfv` and use it in CI without pulling in DuckDB.

### v0.3 — Usable mdvs CLI

Goal: something you can actually use daily for search.

- [ ] `init`, `index`, `search` subcommands via clap
- [ ] `mdvs init` generates `mdvs.toml` (or uses existing), creates DB, downloads model
- [ ] Config-driven field promotion (no interactive prompts — reads from `mdvs.toml`)
- [ ] Dynamic mdfiles table schema based on promoted fields from `mdvs.toml`
- [ ] Incremental indexing with content hashing (per-file diffing, chunk rebuild on change)
- [ ] `index --full` for full rebuild (replaces separate `reindex` command)
- [ ] Model identity storage: model_id, model_dimension, model_revision in vault_meta
- [ ] Model mismatch detection (hard error on ID/dimension mismatch, warning on revision mismatch for search, hard error on revision mismatch for index)
- [ ] `--model` and `--revision` flags on init and as global overrides
- [ ] Handle notes with no frontmatter gracefully
- [ ] `--where` filter on search (SQL expression against promoted columns + metadata)
- [ ] Human-readable table output with chunk-level snippets and heading indicators
- [ ] `mdvs validate` (delegates to mfv engine)
- [ ] `info` command
- [ ] Staleness behavior: `on_stale = "auto"` (incremental sync before search) and `"strict"` modes

### v0.4 — Polish

Goal: comfortable for daily use, handles edge cases.

- [ ] `similar` command (note-to-note or chunk-to-chunk similarity)
- [ ] `query` command (raw SQL passthrough)
- [ ] `export` command (Parquet export of mdfiles, chunks, metadata)
- [ ] `--format json` and `--format paths` output modes
- [ ] Configurable `max_chunk_size` in config and via `--chunk-size` at init
- [ ] `store_raw_content` option in `mdvs.toml`
- [ ] Proper error messages for common failures (vss extension download issues, model download issues, empty directory)
- [ ] Handle edge cases: empty frontmatter, non-UTF8 files, binary files in directory

### v0.5 — Integration

Goal: composable with other tools.

- [ ] `--format json` output suitable for piping into jq / fzf
- [ ] Exit codes for scripting (0 = results found, 1 = no results, 2 = error)
- [ ] Consider: file watcher mode for auto-reindex on changes
- [ ] Consider: MCP server mode for integration with Claude / other AI tools
- [ ] Consider: Obsidian wikilink-aware graph features (backlinks, link graph)
- [ ] Consider: hybrid search (BM25 full-text via DuckDB's fts extension + vector, RRF fusion)
- [ ] Consider: token-based chunk sizing via text-splitter's `tokenizers` feature (for users who want precise token control)

### Non-Goals (at least initially)

- Obsidian plugin (this is a standalone CLI)
- Web UI
- Cloud sync / remote storage
- Chat / RAG / LLM answer generation
- Support for non-Model2Vec embedding backends (Ollama, OpenAI)

## Prior Art

| Project | Stack | Differentiator vs mdvs |
|---|---|---|
| [qmd](https://github.com/tobi/qmd) (Tobias Lütke) | TypeScript/Bun, SQLite, Ollama | More mature, hybrid BM25+vector+rerank. Requires Ollama. |
| [obsidian-note-taking-assistant](https://github.com/sspaeti/obsidian-note-taking-assistant) | Python, DuckDB, BGE-M3 | Closest to our DuckDB approach. Python, heavy model (hours to embed). |
| [mdrag](https://orellazri.com/posts/rag-pipeline-chat-with-my-obsidian-vault/) | Rust, SQLite-vec, Ollama | Rust CLI, but needs Ollama for embeddings. |
| Smart Connections | Obsidian plugin, JS | In-editor, local embeddings. Plugin, not CLI. |

What mdvs offers that none of these do: single static binary with native Rust parsing (no Ollama, no Python, no API keys), instant static embeddings (~30MB model, millisecond inference), DuckDB for both metadata SQL and vector search, user-driven frontmatter promotion instead of hardcoded assumptions, only a single small runtime dependency (the vss extension, downloaded once on init).

## Packaging and Distribution

### What Ships in the Binary

The `mdvs` binary is a single statically-linked Rust executable. Everything compiled from Rust crates (duckdb with `bundled` feature, model2vec-rs, gray_matter, text-splitter, pulldown-cmark, clap, etc.) is baked in at build time. No shared libraries, no runtime interpreters, no system dependencies.

### What Downloads on First Run

`mdvs init` needs network access for two things:

1. **DuckDB vss extension** (~few MB): installed via `INSTALL vss FROM community; LOAD vss;`. DuckDB caches this at `~/.duckdb/extensions/`. Required for HNSW index creation and cosine distance queries.
2. **Embedding model weights** (~30MB for the default potion-multilingual-128M): downloaded from HuggingFace by `model2vec-rs`. Cached at `~/.cache/mdvs/models/`.

After `init` completes, all subsequent operations (`index`, `search`, `similar`, etc.) are fully offline.

The `init` command shows a progress bar (via `indicatif`) for both downloads so the user knows what's happening and how long it will take.

### Distribution Channels

| Channel | Command | Installs |
|---|---|---|
| **crates.io** | `cargo install mdvs` | Full search tool (~20MB) |
| **crates.io** | `cargo install mfv` | Standalone validator (~2MB) |
| **GitHub Releases** | Download pre-built binary for platform | Both binaries per release |
| **Homebrew tap** | `brew install <user>/tap/mdvs` | Full search tool |
| **Homebrew tap** | `brew install <user>/tap/mfv` | Standalone validator |

Pre-built binaries are the primary distribution path. Built in CI via `cargo-dist` or cross-compilation, targeting at minimum: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-apple-darwin`, `aarch64-apple-darwin`. Each release is a single compressed binary — download, extract, put in PATH, done.

### Dependency Comparison

| Tool | Install requires | Runtime requires |
|---|---|---|
| **mdvs** | Download one binary | First-run network for vss extension + model |
| qmd | Node.js/Bun + npm install | Ollama running |
| obsidian-note-taking-assistant | Python + pip/uv + dependencies | Python runtime |
| mdrag | Rust toolchain or binary | Ollama running |

## Open Questions

- **vss extension version compatibility**: does the vss community extension support the same DuckDB version that `duckdb-rs` bundles? Version skew could be a problem.
- **text-splitter heading extraction**: `text-splitter` returns raw markdown chunks without structured heading metadata. Need to validate that a lightweight pulldown-cmark pass over each chunk reliably extracts the relevant heading for the `§ Heading` display. Edge cases: chunks that span across headings, chunks with no headings at all.
- **Dynamic schema generation**: generating `CREATE TABLE mdfiles (...)` dynamically based on promoted field selection at init. Need to handle type inference carefully (especially detecting VARCHAR[] vs VARCHAR for list-valued fields). Also need to handle schema changes if the user re-inits with different promoted fields.
- **Model revision resolution**: need to verify whether `model2vec-rs` exposes the Git commit SHA of the loaded model. If not, fall back to reading it from the HuggingFace cache directory structure (`~/.cache/huggingface/hub/models--org--name/snapshots/<sha>/`).
- **crates.io name availability**: verify that `mdvs` and `mfv` are available on crates.io before publishing.
- **Enum type mapping**: how `enum` fields with `values` map to DuckDB — probably just `VARCHAR` with application-level validation, since DuckDB doesn't have native enum constraints on insert. Validation happens via `mfv check` / `mdvs validate`, not at the DB layer.

### Resolved

- **FLOAT[N] parameter binding**: resolved — embeddings are inserted as SQL literals `[...]::FLOAT[256]` (no `Vec<f32>` ToSql support in duckdb-rs).
- **gray_matter edge cases**: resolved — tested against real vaults; unclosed delimiters and malformed YAML/TOML return `None` gracefully.
- **Config file conflicts**: resolved — no shared `frontmatter.toml`. Each tool has its own file (`mfv.toml` / `mdvs.toml`) with fallback precedence.
- **Promoted field separation**: resolved — `mfv` ignores `promoted` and unknown TOML sections via serde defaults.
- **Path-scoped requirements**: resolved — using `globset` crate for glob matching on `paths` field in validation rules.
