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
| `--limit` / `-n` | `10` | Maximum number of results |
| `--where` | | SQL WHERE clause for filtering on frontmatter fields |
| `--no-update` | | Skip auto-update |
| `--no-build` | | Skip auto-build before searching |

The default limit can be changed in `mdvs.toml` via `[search].default_limit`.

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`search` loads the index from `.mdvs/`, embeds the query into a vector using the same model that built the index, and ranks files by [cosine similarity](../concepts/search.md#scores). Each file's score is the **best chunk match** — the highest similarity across all its chunks. Results are sorted descending (higher = more similar).

By default, `search` auto-builds the index before querying, which includes auto-updating the schema (see [`[search].auto_build`](../configuration.md#search)). Use `--no-build` to query the existing index as-is, or `--no-update` to build without updating the schema first.

See [Search & Indexing](../concepts/search.md) for details on chunking, embedding, scoring, and model identity.

### First run

> **Note:** The very first time `search` (or `build`) runs, mdvs downloads the embedding model from HuggingFace to a local cache. This is a one-time download — subsequent runs use the cached model and start instantly.
>
> Download size depends on the model:
>
> | Model | Size |
> |---|---|
> | `potion-base-2M` | ~8 MB |
> | `potion-base-8M` (default) | ~30 MB |
> | `potion-base-32M` | ~120 MB |
> | `potion-multilingual-128M` | ~480 MB |
>
> After the model is cached, a full build of 500+ files completes in under a second.

### `--where`

Filter results by frontmatter fields using SQL syntax. The filter and similarity ranking are combined in a single query, so files that don't match are excluded efficiently.

Scalar comparisons:

```bash
mdvs search "experiment" --where "status = 'active'"
mdvs search "experiment" --where "sample_count > 20"
mdvs search "experiment" --where "status = 'active' AND priority = 1"
```

Array fields (via DataFusion array functions):

```bash
mdvs search "calibration" --where "array_has(tags, 'biosensor')"
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
│ model                    │ minishlab/potion-base-8M                          │
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
│ model                    │ minishlab/potion-base-8M                          │
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
Load model: minishlab/potion-base-8M (22ms)
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
