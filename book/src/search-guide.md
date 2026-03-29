# Search Guide

The `--where` flag on [search](./commands/search.md) lets you filter results by frontmatter fields using SQL syntax. The filter is combined with similarity ranking in a single query — files that don't match are excluded before results are returned.

Under the hood, mdvs uses [DataFusion](https://datafusion.apache.org/) as its SQL engine, so any expression valid in DataFusion's SQL dialect works in `--where`.

## Scalar fields

Use bare field names for simple comparisons:

### String

```bash
mdvs search "experiment" --where "status = 'active'"
mdvs search "experiment" --where "author = 'Giulia Ferretti'"
mdvs search "experiment" --where "status IN ('active', 'archived')"
mdvs search "experiment" --where "title LIKE '%sensor%'"
```

### Numeric

```bash
mdvs search "experiment" --where "sample_count > 20"
mdvs search "experiment" --where "drift_rate >= 0.01 AND drift_rate <= 0.05"
mdvs search "experiment" --where "wavelength_nm BETWEEN 600 AND 800"
```

```
Searched "experiment" — 2 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ experiment                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/notes/experiment-3.md              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.420                                             │
...

...
```

### Boolean

```bash
mdvs search "announcement" --where "draft = false"
mdvs search "ideas" --where "draft = true"
```

### Null checks

```bash
mdvs search "notes" --where "drift_rate IS NOT NULL"
mdvs search "notes" --where "review_score IS NULL"
```

### Combining conditions

Use `AND`, `OR`, and `NOT` to build compound filters:

```bash
mdvs search "experiment" --where "status = 'active' AND priority = 1"
mdvs search "notes" --where "author = 'REMO' OR author = 'Marco Bianchi'"
mdvs search "notes" --where "NOT status = 'archived'"
```

## Array fields

Fields typed as `String[]` (like `tags`, `attendees`, `action_items`) support array functions.

### Containment

```bash
mdvs search "calibration" --where "array_has(tags, 'calibration')"
```

```
Searched "calibration" — 4 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ calibration                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/notes/experiment-1.md              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.478                                             │
...

...
```

The SQL-standard `ANY` syntax also works:

```bash
mdvs search "calibration" --where "'calibration' = ANY(tags)"
```

### Multiple tags

Combine with `AND` to require multiple values:

```bash
mdvs search "calibration" --where "array_has(tags, 'calibration') AND array_has(tags, 'SPR-A1')"
```

### Array length

```bash
mdvs search "meeting" --where "array_length(action_items) > 2"
```

## Filtering by file path

Filter results by file path using the `filepath` column:

```bash
mdvs search "experiment" --where "filepath LIKE 'projects/alpha/%'"
```

```
Searched "experiment" — 8 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ experiment                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/notes/experiment-3.md              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.420                                             │
...

...
```

File paths are stored as relative paths (e.g., `projects/alpha/notes/experiment-1.md`), so use `LIKE` with `%` for path prefix matching:

```bash
# All blog posts
--where "filepath LIKE 'blog/%'"

# Only published blog posts
--where "filepath LIKE 'blog/published/%'"

# Files in any meetings directory
--where "filepath LIKE '%/meetings/%'"
```

## Nested objects

Fields typed as Object (like `calibration` in `example_kb`) are stored as nested Struct columns. Access nested values with bracket notation:

```bash
mdvs search "sensor" --where "calibration['baseline']['wavelength'] > 600"
```

```
Searched "sensor" — 2 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ sensor                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/notes/experiment-2.md              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.414                                             │
...

...
```

The top-level field name (`calibration`) can be used bare. Only the nested access needs brackets:

```bash
# These are equivalent:
--where "calibration['baseline']['wavelength'] > 600"
--where "_data['calibration']['baseline']['wavelength'] > 600"
```

## Field names with special characters

Some field names need quoting in SQL. The [init](./commands/init.md), [update](./commands/update.md), and [info](./commands/info.md) commands show hints in their output when this applies.

### Spaces

Double-quote the field name:

```bash
mdvs search "query" --where "\"lab section\" = 'optics'"
```

### Single quotes in field names

Also use double-quoting:

```bash
mdvs search "query" --where "\"author's_note\" IS NOT NULL"
```

### Double quotes in field names

Double the double quotes inside the identifier:

```bash
mdvs search "query" --where "\"notes\"\"v2\"\" = true"
```

## String values with special characters

To include a literal single quote inside a string value, double it:

```bash
mdvs search "query" --where "title = 'What''s New?'"
```

mdvs validates quote balance before running the query. If you see "unmatched single quote", check that every `'` in a value is doubled.

## Tips

- **Case sensitivity**: field names and string values are case-sensitive. Use `LOWER()` for case-insensitive matching:
  ```bash
  --where "LOWER(author) = 'giulia ferretti'"
  ```

- **LIKE patterns**: `%` matches any sequence, `_` matches a single character:
  ```bash
  --where "title LIKE 'Project%'"       # starts with "Project"
  --where "title LIKE '%sensor%'"       # contains "sensor"
  ```

- **NULL semantics**: comparisons against NULL always return false. Use `IS NULL` / `IS NOT NULL`, not `= NULL`.

- **No aggregates in --where**: functions like `COUNT()` or `SUM()` don't work in `--where` — the filter applies per-file, not across results.
