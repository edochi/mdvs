# init

Scan a directory, infer a typed schema, and optionally build the search index.

## Usage

```bash
mdvs init [path] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory to scan |
| `--glob` | `**` | Glob pattern for matching markdown files |
| `--force` | | Overwrite existing `mdvs.toml` |
| `--dry-run` | | Preview the inferred schema without writing anything |
| `--ignore-bare-files` | | Exclude files without YAML frontmatter |
| `--suppress-auto-build` | | Write `mdvs.toml` only — don't build the search index |
| `--skip-gitignore` | | Don't read `.gitignore` patterns during scan |
| `--model` | `minishlab/potion-base-8M` | HuggingFace model ID for embeddings |
| `--revision` | | Pin model to a specific revision (commit SHA) |
| `--chunk-size` | `1024` | Maximum chunk size in characters |

`--model`, `--revision`, and `--chunk-size` only take effect when auto-build is enabled (the default). They're rejected with `--suppress-auto-build`.

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`init` scans every markdown file in the directory, extracts YAML frontmatter, infers a typed schema with path patterns, and writes `mdvs.toml`. By default it also builds the search index (embedding model download, chunking, vector storage in `.mdvs/`).

See [Getting Started](../getting-started.md) for a full walkthrough with output, and [Schema Inference](../concepts/schema.md) for how types and path patterns are computed.

This creates up to two artifacts:

- **`mdvs.toml`** — the schema file. Commit this to version control.
- **`.mdvs/`** — the search index (Parquet files). Add to `.gitignore`. Only created when auto-build is enabled (the default).

If `mdvs.toml` already exists, `init` refuses to run unless you pass `--force`. To update an existing schema without overwriting it, use [update](./update.md) instead.

### `init --force` vs `update --reinfer-all`

Both re-infer the schema from scratch, but they differ in scope:

- `init --force` overwrites the entire `mdvs.toml` — all sections, including `[scan]`, `[embedding_model]`, `[chunking]`, and `[search]`. Any manual edits are lost.
- `update --reinfer-all` re-infers only the `[fields]` section. All other config is preserved.

### `--suppress-auto-build`

Writes `mdvs.toml` with only the validation sections (`[scan]`, `[update]`, `[fields]`). No `[embedding_model]`, `[chunking]`, or `[search]` sections are written, and no index is built.

Use this when you only need schema validation (`mdvs check`) and don't plan to use search. You can always run `mdvs build` later — it will add the missing sections with defaults.

## Output

### Compact (default)

```bash
mdvs init example_kb --suppress-auto-build
```

```
Initialized 43 files — 37 field(s)

╭─────────────────────┬───────────────────────┬───────┬────────────────────────╮
│ "action_items"      │ String[]              │ 9/43  │                        │
│ "algorithm"         │ String                │ 2/43  │                        │
│ "ambient_humidity"  │ Float                 │ 1/43  │                        │
│ ...                 │                       │       │                        │
│ "drift_rate"        │ Float?                │ 3/43  │                        │
│ ...                 │                       │       │                        │
│ "lab section"       │ String                │ 4/43  │ use "field name" in -- │
│                     │                       │       │ where                  │
│ ...                 │                       │       │                        │
│ "title"             │ String                │ 37/43 │                        │
│ "wavelength_nm"     │ Float                 │ 3/43  │                        │
╰─────────────────────┴───────────────────────┴───────┴────────────────────────╯

Initialized mdvs in 'example_kb'
```

Each row shows the field name, inferred type, how many files contain it (e.g., `9/43`), and optional hints for `--where` syntax (see [Search Guide](../search-guide.md) for details on quoting and escaping). The `?` suffix on a type (e.g., `Float?`) means the field is nullable.

### Verbose (`-v`)

```bash
mdvs init example_kb --suppress-auto-build -v
```

```
Initialized 43 files — 37 field(s)

╭────────────────────────────────┬────────────────────────┬────────────────────╮
│ "action_items"                 │ String[]               │ 9/43               │
├────────────────────────────────┴────────────────────────┴────────────────────┤
│   required:                                                                  │
│     - "meetings/all-hands/**"                                                │
│     - "projects/alpha/meetings/**"                                           │
│     - "projects/beta/meetings/**"                                            │
│   allowed:                                                                   │
│     - "meetings/**"                                                          │
│     - "projects/alpha/meetings/**"                                           │
│     - "projects/beta/meetings/**"                                            │
╰──────────────────────────────────────────────────────────────────────────────╯
╭───────────────────────────────────┬─────────────────────┬────────────────────╮
│ "ambient_humidity"                │ Float               │ 1/43               │
├───────────────────────────────────┴─────────────────────┴────────────────────┤
│   allowed:                                                                   │
│     - "projects/alpha/notes/**"                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────────┬──────────────────────────┬────────────────────╮
│ "drift_rate"                 │ Float?                   │ 3/43               │
├──────────────────────────────┴──────────────────────────┴────────────────────┤
│   required:                                                                  │
│     - "projects/alpha/notes/**"                                              │
│   allowed:                                                                   │
│     - "projects/alpha/notes/**"                                              │
│   nullable: true                                                             │
╰──────────────────────────────────────────────────────────────────────────────╯
...
```

Verbose output shows each field as a record with its `required` and `allowed` glob patterns. Fields with `required = []` omit the required line. Nullable fields show `nullable: true`.

## Examples

### Preview the schema

Use `--dry-run` to see what `init` would infer without writing anything:

```bash
mdvs init example_kb --dry-run --force
```

Nothing is written — the output shows the same discovery table, followed by `(dry run, nothing written)`.

### Exclude bare files

By default, files without frontmatter are included in the scan. This affects field counts — a bare file at the root means `title` appears in 37/43 files instead of 37/37:

```bash
mdvs init example_kb --dry-run --force --ignore-bare-files
```

```
Initialized 37 files — 37 field(s) (dry run)

╭─────────────────────┬───────────────────────┬───────┬────────────────────────╮
│ ...                 │                       │       │                        │
│ "title"             │ String                │ 37/37 │                        │
│ ...                 │                       │       │                        │
╰─────────────────────┴───────────────────────┴───────┴────────────────────────╯
```

With `--ignore-bare-files`, only 37 files are scanned and `title` becomes 37/37. This also affects the inferred `required` patterns — without bare files diluting the counts, more fields can be required in broader paths.

### Schema only, no index

```bash
mdvs init example_kb --suppress-auto-build
```

Writes `mdvs.toml` but skips the embedding step. No `.mdvs/` directory is created. Useful when you only need `mdvs check` for frontmatter validation.

## Errors

| Error | Cause |
|---|---|
| `mdvs.toml already exists` | Config exists and `--force` not passed |
| `is not a directory` | Path doesn't exist or isn't a directory |
| `no markdown files found` | No `.md` files match the glob pattern |
| `--model has no effect without --auto-build` | Build flags used with `--suppress-auto-build` |
