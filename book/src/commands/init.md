# init

Scan a directory, infer a typed schema, and write `mdvs.toml`.

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
| `--skip-gitignore` | | Don't read `.gitignore` patterns during scan |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`init` scans every markdown file, extracts YAML frontmatter, infers a typed schema with path patterns, and writes `mdvs.toml`. It does not build the search index — run [build](./build.md) or [search](./search.md) for that.

See [Getting Started](../getting-started.md) for a full walkthrough with output, and [Schema Inference](../concepts/schema.md) for how types and path patterns are computed.

One artifact is created: **`mdvs.toml`** — the schema file. Commit this to version control.

If `mdvs.toml` or `.mdvs/` already exists, `init` refuses to run unless you pass `--force`. With `--force`, both `mdvs.toml` and `.mdvs/` are deleted before proceeding. To update an existing schema without overwriting it, use [update](./update.md) instead.

### `init --force` vs `update --reinfer-all`

Both re-infer the schema from scratch, but they differ in scope:

- `init --force` overwrites the entire `mdvs.toml` — all sections, including `[scan]`, `[fields]`, and any build sections. Any manual edits are lost. `.mdvs/` is also deleted.
- `update --reinfer-all` re-infers only the `[fields]` section. All other config is preserved.

## Output

### Compact (default)

```bash
mdvs init example_kb
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
mdvs init example_kb -v
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

## Errors

| Error | Cause |
|---|---|
| `mdvs.toml already exists` | Config exists and `--force` not passed |
| `is not a directory` | Path doesn't exist or isn't a directory |
| `no markdown files found` | No `.md` files match the glob pattern |
