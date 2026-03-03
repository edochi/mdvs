# Searching

mdvs provides semantic search — it finds files by meaning, not just keyword matching. This page covers filtering, output formats, and tips for getting the best results.

## Basic search

```bash
mdvs search "how does error handling work in rust"
```

```
0.72  blog/rust-errors.md
0.58  blog/async-patterns.md
0.31  ideas.md
```

Each result shows a similarity score (0–1) and the file path. Results are ranked by the highest-scoring chunk in each file.

## Filtering with --where

Use `--where` to filter results on frontmatter fields. The syntax is SQL:

```bash
# Boolean fields
mdvs search "rust" --where "draft = false"

# String comparison
mdvs search "notes" --where "author = 'Alice'"

# Null checks
mdvs search "recipes" --where "tags IS NOT NULL"

# Numeric comparison
mdvs search "papers" --where "year >= 2024"

# Combine with AND/OR
mdvs search "rust" --where "draft = false AND year >= 2024"
```

### Array fields

For array fields like `tags: [rust, traits]`, use DataFusion's array functions:

```bash
# Files where tags contains 'rust'
mdvs search "ownership" --where "array_has(tags, 'rust')"

# Files with more than 2 tags
mdvs search "notes" --where "array_length(tags) > 2"
```

Field names are used directly — no prefix or special syntax needed. Under the hood, mdvs uses [DataFusion](https://datafusion.apache.org/) for SQL execution, so any valid SQL expression works.

## Limiting results

```bash
mdvs search "rust" --limit 5
```

The default limit is set in `mdvs.toml` under `[search].default_limit` (default: 10).

## JSON output

All commands support `--output json` for scripting and pipelines:

```bash
mdvs search "rust" --output json
```

```json
{
  "hits": [
    { "filename": "blog/rust-errors.md", "score": 0.72 },
    { "filename": "blog/async-patterns.md", "score": 0.58 }
  ]
}
```

This works with `jq`, scripts, or any tool that consumes JSON:

```bash
# Get just the filenames
mdvs search "rust" --output json | jq -r '.hits[].filename'

# Open the top result
mdvs search "rust" --output json | jq -r '.hits[0].filename' | xargs open
```

## How scoring works

mdvs splits each file into semantic chunks (respecting markdown structure like headings and paragraphs), embeds each chunk, and computes cosine similarity between your query and every chunk.

A file's score is the **maximum** chunk similarity — not the average. This means a long file with one highly relevant section ranks above a file with uniformly mediocre relevance.

## Rebuilding the index

The search index is built from your markdown files and stored in `.mdvs/`. It needs to be rebuilt when file contents change:

```bash
mdvs build
```

Builds are **incremental** by default — only new and changed files are re-embedded. If nothing changed, the model isn't even loaded.

```
Built index: 2 new, 1 edited, 443 unchanged, 0 removed (3 files embedded)
```

Use `--force` for a full rebuild:

```bash
mdvs build --force
```

A full rebuild is required when you change the model, chunk size, or internal prefix.

## Tips

- **Be specific.** "rust error handling patterns" works better than just "errors".
- **Use --where to narrow results.** If you know the field values, filtering is faster and more precise than relying on semantic similarity alone.
- **Combine search and check.** Run `mdvs check` to ensure your frontmatter is clean before relying on `--where` filters — a misspelled boolean won't match `draft = false`.
