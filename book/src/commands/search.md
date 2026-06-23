# search

Query the index with natural language.

## Usage

```bash
mdvs search <query> [path] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `query` | (required) | Natural language search query |
| `path` | `.` | Directory containing `mdvs.toml` |
| `--mode` | `hybrid` | Search mode: `semantic`, `fulltext`, or `hybrid` |
| `--limit` / `-n` | `10` | Maximum number of results |
| `--where` | | SQL WHERE clause — filter on frontmatter fields or on the `filepath` column |
| `--no-update` | | Skip auto-update |
| `--no-build` | | Skip auto-build before searching |

The default limit can be changed in `mdvs.toml` via `[search].default_limit`.

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`search` loads the Lance index from `.mdvs/`, runs the query through LanceDB, and ranks files by their best-matching chunk. The exact ranking depends on `--mode`:

- **`semantic`** — embed the query with the same model that built the index, cosine-rank chunks against `embedding`.
- **`fulltext`** — BM25 rank chunks against the persisted `chunk_text` (no model load needed).
- **`hybrid`** (default) — run both and combine with LanceDB's Reciprocal Rank Fusion reranker.

Each file's score is the **best chunk match** across all its chunks (see [scoring](../concepts/search.md#scores)). Results are sorted descending (higher = better match).

By default, `search` auto-builds the index before querying, which includes auto-updating the schema (see [`[search].auto_build`](../configuration.md#search)). The chain is cheap on unchanged corpora — update is fast, classify sees no work, and the Lance write is skipped. Use `--no-build` to query a pre-built index without touching it (deterministic CI search), or `--no-update` to build against the committed schema.

See [Search & Indexing](../concepts/search.md) for details on chunking, embedding, scoring, and model identity.

### First run

> **Note:** The very first time `search` (or `build`) runs, mdvs downloads the embedding model from HuggingFace to a local cache. This is a one-time download — subsequent runs use the cached model and start instantly.
>
> Download size depends on the model:
>
> | Model | Size |
> |---|---|
> | `potion-base-2M` | ~8 MB |
> | `potion-base-8M` | ~30 MB |
> | `potion-base-32M` | ~120 MB |
> | `potion-multilingual-128M` (default) | ~480 MB |
>
> After the model is cached, a full build of 500+ files completes in under a second.

### `--where`

Filter results using SQL syntax. The filter and similarity ranking are combined in a single query, so files that don't match are excluded efficiently. `--where` operates on **any column** in the Lance index — frontmatter fields (auto-discovered from `mdvs.toml`) and the always-present `filepath` column.

Scalar frontmatter comparisons:

```bash
mdvs search "experiment" --where "status = 'active'"
mdvs search "experiment" --where "sample_count > 20"
mdvs search "experiment" --where "status = 'active' AND priority = 1"
```

Array fields — `=` / `!=` / `IN` / `NOT IN` are auto-rewritten to `array_has(...)`:

```bash
mdvs search "calibration" --where "tags = 'biosensor'"               # auto-rewritten
mdvs search "calibration" --where "tags IN ('biosensor', 'optics')"  # OR-chain of array_has
```

The translation note at the top of the result shows the rewrite. `--where` clauses that reference `Array(Float)` fields are rejected up front with a clear error — see the [Search Guide](../search-guide.md) for the workaround.

Path filtering via the always-present `filepath` column (its last component is the filename):

```bash
mdvs search "race condition" --where "filepath LIKE 'logs/%'"        # everything under logs/
mdvs search "review" --where "filepath LIKE '%-postmortem.md'"       # filename suffix
mdvs search "alpha" --where "filepath = 'projects/alpha/overview.md'" # exact path
mdvs search "deploy" --where "filepath LIKE 'logs/%' AND status = 'published'"  # combine
```

Field names with spaces need double-quoting:

```bash
mdvs search "query" --where "\"lab section\" = 'optics'"
```

See [Search Guide](../search-guide.md) for the full `--where` reference, including nested objects, escaping rules, and more examples.

## Output

### Compact (default)

```bash
mdvs search "experiment" example_kb -n 3
```

A header table shows the query metadata, followed by one key-value table per hit numbered `#1`, `#2`, etc. Each hit includes the file, similarity score, line range, and the best-matching chunk text:

```
Searched "experiment" — 3 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ experiment                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-multilingual-128M               │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 3                                                 │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/archived/gamma/lessons-learned.md        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.487                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ lines                    │ 26-28                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ text                     │ ## On REMO                                        │
│                          │                                                   │
│                          │ REMO's environmental monitoring data from the out │
│                          │ door tests was the most useful output of the enti │
│                          │ re project. ...                                   │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #2 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ blog/published/2031/founding-story.md             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.470                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ lines                    │ 21-21                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ text                     │ We are a small lab and we intend to stay small... │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #3 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/archived/gamma/post-mortem.md            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.457                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ lines                    │ 11-21                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ text                     │ # Project Gamma — Post-Mortem ...                 │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

With `--where` filtering, only files matching the SQL clause are included:

```bash
mdvs search "experiment" example_kb --where "status = 'active'" -n 5
```

```
Searched "experiment" — 3 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ experiment                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-multilingual-128M               │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 5                                                 │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/overview.md                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.391                                             │
...
```

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result:

```bash
mdvs search "experiment" example_kb -v -n 3
```

```
Read config: example_kb/mdvs.toml (2ms)
Scan: 43 files (2ms)
...
Load model: minishlab/potion-multilingual-128M (22ms)
Embed query: "experiment" (0ms)
Execute search: 3 hits (5ms)
Searched "experiment" — 3 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ experiment                                        │
...
```

The hit tables are identical in both modes — verbose only adds the step lines showing processing times.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Search completed (even with 0 results) |
| `2` | Pipeline error (missing config, missing index, model mismatch, invalid `--where`) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `index not found` | `.mdvs/` doesn't exist — run `mdvs build` first |
| `model mismatch` | Config model differs from index — run `mdvs build` to rebuild |
| Invalid `--where` | SQL syntax error or unknown field name |
