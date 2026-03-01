# `mdvs search`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md), [build](build.md)

---

## Synopsis

```
mdvs search <query> [path] [flags]
```

| Flag       | Type       | Default       | Description                             |
|------------|------------|---------------|-----------------------------------------|
| `query`    | positional | (required)    | Natural language search query            |
| `path`     | positional | `.`           | Directory containing mdvs.toml          |
| `--limit`  | usize      | (from toml)   | Max results (`[search].default_limit`)  |
| `--where`  | string     | (none)        | SQL WHERE clause on frontmatter fields  |

---

## Behavior

1. Read `mdvs.toml` (see [Prerequisites](check.md#prerequisites))
2. Check `.mdvs/` exists with parquet files (see [Errors](#errors))
3. Read parquet metadata, compare model against toml:
   - Model name or revision mismatch → hard error
4. Load model
5. Embed query
6. Compute cosine similarity against all chunk embeddings
7. Note-level ranking: max chunk similarity per file
8. Apply `--where` filter via DataFusion SQL on frontmatter fields
9. Apply `--limit`
10. Collect `SearchResult`
11. Print result

Progress messages ("Loading model...") go to **stderr**.
The formatted result goes to **stdout**.

### Note-level ranking

Each file may have multiple chunks. The file's score is the **maximum** chunk similarity
across its chunks (not average). This ensures a file with one highly relevant section
ranks above a file with uniformly mediocre relevance.

### WHERE clause

The `--where` flag accepts a raw SQL expression applied to the `data` Struct column
in `files.parquet`. Field access uses the column names directly.

```bash
mdvs search "rust async" --where "tags = 'rust' AND draft = false"
```

---

## Output

```rust
#[derive(Serialize)]
pub struct SearchResult {
    pub query: String,
    pub hits: Vec<SearchHit>,
    pub total_files: usize,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub file: PathBuf,
    pub score: f32,
}
```

### Human format

```
 Score  File
 ──────────────────────────
 0.87   blog/rust.md
 0.82   blog/cli.md
 0.71   notes/ideas.md

3 results (12 files searched)
```

### JSON format

```json
{
  "query": "rust async",
  "hits": [
    { "file": "blog/rust.md", "score": 0.87 },
    { "file": "blog/cli.md", "score": 0.82 },
    { "file": "notes/ideas.md", "score": 0.71 }
  ],
  "total_files": 12
}
```

---

## Errors

| Condition                  | Message                                                                     |
|----------------------------|-----------------------------------------------------------------------------|
| No `.mdvs/` directory     | `no index found in '<path>' — run 'mdvs build' to create one`              |
| Model name mismatch        | `index was built with model '<old>', but mdvs.toml specifies '<new>' — rebuild the index or revert the model` |
| Model revision mismatch    | `index was built with revision '<old>', but mdvs.toml specifies '<new>' — rebuild the index or revert the revision` |
| Invalid WHERE clause       | `invalid --where clause: <SQL parse error>`                                 |
| No results                 | (not an error — print empty result with 0 results)                          |

See also [Prerequisites](check.md#prerequisites) for toml validation errors.

---

## TODOs

- **Staleness detection:** warn or auto-rebuild if files changed since last build
- **Frontmatter in output:** optionally show frontmatter values in results table
- **Snippets:** show matching chunk text

---

## Examples

```bash
# Basic search
mdvs search "how to handle errors in rust"

# Limit results
mdvs search "async patterns" --limit 5

# Filter by frontmatter
mdvs search "testing" --where "tags = 'rust'"

# JSON output (global flag)
mdvs search "authentication" --output json

# Search a specific directory
mdvs search "deployment" ~/notes
```
