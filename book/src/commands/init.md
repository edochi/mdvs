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

### `init --force` vs `update reinfer`

Both re-infer the schema from scratch, but they differ in scope:

- `init --force` overwrites the entire `mdvs.toml` — all sections, including `[scan]`, `[fields]`, and any build sections. Any manual edits are lost. `.mdvs/` is also deleted.
- `update reinfer` re-infers only the `[fields]` section. All other config is preserved.

## Output

### Compact (default)

```bash
mdvs init example_kb
```

Each discovered field is shown as its own key-value table with the field name on the top border. Only a few fields are shown here — the full output includes all 37:

```
Initialized 43 files — 37 field(s)

┌ action_items ────────────┬───────────────────────────────────────────────────┐
│ type                     │ String[]                                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 9 out of 43                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ meetings/all-hands/**                             │
│                          │ projects/alpha/meetings/**                        │
│                          │ projects/beta/meetings/**                         │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ meetings/**                                       │
│                          │ projects/alpha/meetings/**                        │
│                          │ projects/beta/meetings/**                         │
└──────────────────────────┴───────────────────────────────────────────────────┘

...

┌ drift_rate ──────────────┬───────────────────────────────────────────────────┐
│ type                     │ Float                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 3 out of 43                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ true                                              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ projects/alpha/notes/**                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ projects/alpha/notes/**                           │
└──────────────────────────┴───────────────────────────────────────────────────┘

...

┌ title ───────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 37 out of 43                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ blog/**                                           │
│                          │ meetings/**                                       │
│                          │ people/**                                         │
│                          │ projects/**                                       │
│                          │ reference/protocols/**                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ blog/**                                           │
│                          │ meetings/**                                       │
│                          │ people/**                                         │
│                          │ projects/**                                       │
│                          │ reference/protocols/**                            │
└──────────────────────────┴───────────────────────────────────────────────────┘

Initialized mdvs in 'example_kb'
```

Each table shows the inferred type, file count, nullable status, and inferred `required`/`allowed` glob patterns. Fields with special characters in their name (e.g., `lab section`) include a `hints` row with `--where` syntax advice (see [Search Guide](../search-guide.md)).

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result:

```bash
mdvs init example_kb -v
```

```
Scan: 43 files (5ms)
Infer: 37 field(s) (0ms)
Write config: example_kb/mdvs.toml (0ms)
Initialized 43 files — 37 field(s)

┌ action_items ────────────┬───────────────────────────────────────────────────┐
│ type                     │ String[]                                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 9 out of 43                                       │
...
```

The field tables are identical in both modes — verbose only adds the step lines showing processing times.

## Examples

### Preview the schema

Use `--dry-run` to see what `init` would infer without writing anything:

```bash
mdvs init example_kb --dry-run --force
```

Nothing is written — the output shows the same discovery table, followed by `(dry run, nothing written)`.

### Exclude bare files

By default, files without frontmatter are included in the scan. This affects field counts — a bare file at the root means `title` appears in `37 out of 43` files instead of `37 out of 37`:

```bash
mdvs init example_kb --dry-run --force --ignore-bare-files
```

With `--ignore-bare-files`, only 37 files are scanned. The `files` row for `title` becomes `37 out of 37`. This also affects the inferred `required` patterns — without bare files diluting the counts, more fields can be required in broader paths.

## Errors

| Error | Cause |
|---|---|
| `mdvs.toml already exists` | Config exists and `--force` not passed |
| `is not a directory` | Path doesn't exist or isn't a directory |
| `no markdown files found` | No `.md` files match the glob pattern |
