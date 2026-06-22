# Search Guide

The `--where` flag on [search](./commands/search.md) lets you filter results by frontmatter fields using SQL syntax. The filter is combined with similarity ranking in a single query — files that don't match are excluded before results are returned.

Under the hood, mdvs hands the clause to [LanceDB](https://lancedb.com/)'s SQL filter, which is built on top of DataFusion — so any expression valid in DataFusion's SQL dialect works in `--where`.

> **Limitation.** `--where` clauses that reference an `Array(Float)` field (e.g. `measurement_values`) are rejected up front, because the underlying search engine can't safely decode them and crashes on read. mdvs catches this before the query runs and returns a clear error. Filter on a scalar field, or store the data as a parallel array of strings, instead.

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
│ model                    │ minishlab/potion-multilingual-128M               │
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

## Date and DateTime

Fields typed as `Date` (Arrow `Date32`) and `DateTime` (Arrow `Timestamp(Millisecond, UTC)`) support native date arithmetic, comparisons, and the usual SQL date functions. Auto-inferred from RFC 3339 strings — see [Date and DateTime](./concepts/types.md#date-and-datetime) for the type itself.

### Direct comparison

```bash
mdvs search "researcher" --where "joined > '2024-01-01'"
mdvs search "meeting" --where "date < '2032-01-01'"
mdvs search "calibration" --where "synced_at >= '2024-04-01T00:00:00Z'"
```

DateTime offsets are normalized to UTC at storage time, so `2024-04-02T16:14:30+02:00` (in a YAML file) and `2024-04-02T14:14:30Z` (in a `--where` clause) compare as the same absolute moment.

### Range filters (`BETWEEN`)

```bash
mdvs search "meeting" --where "date BETWEEN '2031-09-01' AND '2031-11-30'"
mdvs search "report" --where "joined BETWEEN '2023-01-01' AND '2024-12-31'"
```

### Date functions (`EXTRACT`, `date_part`)

Both extract numeric components from `Date` and `DateTime`. Two equivalent syntaxes:

```bash
mdvs search "meeting" --where "EXTRACT(YEAR FROM date) = 2031"
mdvs search "meeting" --where "date_part('year', date) = 2031"
mdvs search "meeting" --where "EXTRACT(MONTH FROM date) = 10"
mdvs search "calibration" --where "EXTRACT(YEAR FROM synced_at) = 2024 AND EXTRACT(MONTH FROM synced_at) <= 3"
```

### Date arithmetic with `INTERVAL`

The SQL engine supports adding/subtracting intervals to dates and datetimes.

```bash
# Joined within the last 2 years (relative to a cutoff date)
mdvs search "researcher" --where "joined > CAST('2032-01-01' AS DATE) - INTERVAL '2 years'"

# Datetime offset by days
mdvs search "experiment" \
  --where "synced_at < CAST('2024-04-15T00:00:00Z' AS TIMESTAMP) - INTERVAL '7 days'"
```

`CAST('...' AS DATE)` and `CAST('...' AS TIMESTAMP)` are usually needed for string literals on the right side of the arithmetic — the SQL type inference doesn't always pick the date/timestamp type automatically.

### Date subtraction (days between)

Subtracting two `Date` values returns a number of days (an integer):

```bash
# People who joined more than 365 days before a cutoff
mdvs search "researcher" --where "CAST('2032-01-01' AS DATE) - joined > 365"
```

### Null checks

`Date` and `DateTime` columns support standard null predicates, including for fields scoped to a subset of directories (rows outside the scope have null values for that column):

```bash
mdvs search "protocol" --where "last_reviewed IS NOT NULL"
mdvs search "experiment" \
  --where "drift_rate IS NULL AND filepath LIKE 'projects/alpha/notes/%'"
```

### Combining with other filters

Date filters compose freely with the rest of the language — string compare, `IN`, `LIKE`, dotted-leaf access, array operations, and search ranking:

```bash
# Blog posts in 2031 H2 by specific authors
mdvs search "research" \
  --where "filepath LIKE 'blog/published/%' AND author IN ('Marco Bianchi', 'Giulia Ferretti') AND date BETWEEN '2031-07-01' AND '2031-12-31'"

# High-or-medium priority experiments with baseline > 700nm synced in 2024
mdvs search "experiment SPR" \
  --where "(priority = 'high' OR priority = 'medium') AND calibration.baseline.wavelength > 700 AND EXTRACT(YEAR FROM synced_at) = 2024"
```

## Array fields

Fields typed as `Array(String)` (like `tags`, `attendees`, `action_items`) support array functions.

### Containment

```bash
mdvs search "calibration" --where "array_has(tags, 'calibration')"
```

```
Searched "calibration" — 4 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ calibration                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-multilingual-128M               │
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
│ model                    │ minishlab/potion-multilingual-128M               │
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
│ model                    │ minishlab/potion-multilingual-128M               │
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
