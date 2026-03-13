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

The default limit can be changed in `mdvs.toml` via `[search].default_limit`.

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`search` loads the index from `.mdvs/`, embeds the query into a vector using the same model that built the index, and ranks files by [cosine similarity](../concepts/search.md#scores). Each file's score is the **best chunk match** — the highest similarity across all its chunks. Results are sorted descending (higher = more similar).

See [Search & Indexing](../concepts/search.md) for details on chunking, embedding, scoring, and model identity.

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
mdvs search "experiment" example_kb
```

```
Searched "experiment" — 10 hits

╭───────────┬────────────────────────────────────────────────────┬─────────────╮
│ 1         │ "projects/archived/gamma/lessons-learned.md"       │ 0.487       │
│ 2         │ "blog/published/2031/founding-story.md"            │ 0.470       │
│ 3         │ "projects/archived/gamma/post-mortem.md"           │ 0.457       │
│ 4         │ "projects/alpha/notes/experiment-3.md"             │ 0.420       │
│ 5         │ "blog/drafts/grant-ideas.md"                       │ 0.406       │
│ ...       │                                                    │             │
╰───────────┴────────────────────────────────────────────────────┴─────────────╯
```

Each row shows rank, filename, and cosine similarity score.

With `--where` filtering:

```bash
mdvs search "experiment" example_kb --where "status = 'active'" -n 5
```

```
Searched "experiment" — 3 hits

╭───────────────┬──────────────────────────────────────────┬───────────────────╮
│ 1             │ "projects/alpha/overview.md"             │ 0.391             │
│ 2             │ "projects/beta/overview.md"              │ 0.358             │
│ 3             │ "projects/alpha/budget.md"               │ 0.001             │
╰───────────────┴──────────────────────────────────────────┴───────────────────╯
```

### Verbose (`-v`)

```bash
mdvs search "experiment" example_kb -v -n 3
```

```
Searched "experiment" — 3 hits

╭──────────┬─────────────────────────────────────────────────────┬─────────────╮
│ 1        │ "projects/archived/gamma/lessons-learned.md"        │ 0.487       │
├──────────┴─────────────────────────────────────────────────────┴─────────────┤
│   lines 17-19:                                                               │
│                                                                              │
│     ## On Timelines                                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
╭────────────┬─────────────────────────────────────────────────┬───────────────╮
│ 2          │ "blog/published/2031/founding-story.md"         │ 0.470         │
├────────────┴─────────────────────────────────────────────────┴───────────────┤
│   lines 11-11:                                                               │
│     # How Prismatiq Started                                                  │
╰──────────────────────────────────────────────────────────────────────────────╯
╭───────────┬──────────────────────────────────────────────────┬───────────────╮
│ 3         │ "projects/archived/gamma/post-mortem.md"         │ 0.457         │
├───────────┴──────────────────────────────────────────────────┴───────────────┤
│   lines 1-11:                                                                │
│     ---                                                                      │
│     title: "Project Gamma — Post-Mortem"                                     │
│     ...                                                                      │
╰──────────────────────────────────────────────────────────────────────────────╯
3 hits | model: "minishlab/potion-base-8M" | limit: 10
```

Verbose output expands each result into a record showing the best-matching chunk text with its line range. The footer shows total hits, model name, and limit.

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
