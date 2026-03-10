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

The `--where` flag accepts a raw SQL expression on frontmatter fields.
Field names are used directly — no prefix or bracket notation needed.

```bash
mdvs search "rust async" --where "draft = false"
```

#### Scalar fields

Standard SQL comparisons work on scalar fields (String, Integer, Float, Boolean):

```bash
# String equality
--where "status = 'published'"

# Numeric comparison
--where "sample_count > 20"

# Boolean
--where "draft = false"

# Combined
--where "status = 'active' AND sample_count >= 16"
```

#### Array fields

Array fields (e.g. `tags: String[]`) support containment queries via DataFusion's built-in array functions:

```bash
# Check if array contains a value
--where "array_has(tags, 'calibration')"

# SQL standard syntax (equivalent)
--where "'calibration' = ANY(tags)"

# Multiple containment checks (AND = all must match)
--where "array_has(tags, 'calibration') AND array_has(tags, 'SPR-A1')"

# Array length
--where "array_length(tags) > 2"

# Combined with scalar filter
--where "array_has(tags, 'calibration') AND status = 'completed'"
```

#### Nested object fields

Object fields promoted through the view are accessible as Struct columns. Use bracket notation to reach nested children:

```bash
# Access nested field
--where "calibration['baseline']['wavelength'] = 632.8"

# Combine with other filters
--where "calibration['adjusted']['intensity'] > 0.96 AND sensor_type = 'SPR-B2'"
```

#### Field names with special characters

Field names containing spaces require double-quoted SQL identifiers. Field names with single quotes use `''` escaping in bracket accessors. `mdvs info` shows hints for fields that need special quoting.

```bash
# Field with spaces (double-quote the identifier)
--where "\"my field\" = 'value'"

# Field with single quote (escaped in accessor)
--where "\"author's_note\" IS NOT NULL"
```

#### Legacy bracket syntax

The old bracket syntax on the raw `_data` column still works:

```bash
--where "_data['draft'] = false"
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

All examples use `example_kb/` (Prismatiq Lab fixture).

```bash
# Basic search
mdvs search "calibration drift" example_kb

# Limit results
mdvs search "humidity sensor" example_kb --limit 5

# Filter by scalar field
mdvs search "experiment results" example_kb --where "status = 'completed'"

# Filter by boolean
mdvs search "equipment" example_kb --where "draft = false"

# Filter by array containment
mdvs search "calibration" example_kb --where "array_has(tags, 'SPR-A1')"

# Multiple array checks
mdvs search "sensor" example_kb --where "array_has(tags, 'calibration') AND array_has(tags, 'environment')"

# Nested object query
mdvs search "wavelength sweep" example_kb --where "calibration['baseline']['wavelength'] = 632.8"

# Combined scalar + array
mdvs search "results" example_kb --where "array_has(tags, 'calibration') AND status = 'completed'"

# JSON output (global flag)
mdvs search "metamaterial" example_kb --output json

# Search specific directory
mdvs search "deployment" ~/notes
```
