# update

Re-scan files, infer new fields, and update the schema.

## Usage

```bash
mdvs update [path] [--dry-run]
mdvs update [path] reinfer [fields..] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--dry-run` | | Preview changes without writing anything |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`update` re-scans the directory using the existing `[scan]` config, infers types and path patterns from the current files, and merges the results into `mdvs.toml`. Unlike [init](./init.md), it preserves all existing configuration — only the `[fields]` section changes.

### Default mode

By default, `update` only discovers **new** fields — fields that appear in frontmatter but aren't yet in `mdvs.toml` (either as `[[fields.field]]` entries or in the `ignore` list). Existing fields are protected: their types, allowed/required patterns, nullable flags, and constraints don't change.

Fields that disappear (no longer in any file) are kept in `mdvs.toml` by default. This is conservative — removing a field from the schema is an explicit action.

### `reinfer` subcommand

Re-infer field definitions from scratch. This is a subcommand of `update` with its own flags:

| Flag | Description |
|---|---|
| `fields..` | Fields to reinfer (all if none specified) |
| `--with <kinds>` | Comma-separated constraint kinds to apply (`categorical`, `range`, `none`). Requires named fields. |
| `--max-categories <N>` | Override max distinct values for categorical inference |
| `--min-repetition <N>` | Override min average repetition for categorical inference |
| `--dry-run` | Preview changes without writing anything |

**Reinfer specific fields:**

```bash
mdvs update example_kb reinfer drift_rate priority
```

The named fields are removed from `mdvs.toml` and re-inferred from scratch, as if they'd never been seen. All other fields stay protected. Fails if a named field isn't in `mdvs.toml`.

Without `--with`, reinfer applies the default heuristic (categorical detection — see [Constraints](../concepts/constraints.md)). Use `--with` to override:

```bash
# Force categorical (skip heuristic threshold)
mdvs update example_kb reinfer title --with=categorical

# Infer min/max from observed numeric values
mdvs update example_kb reinfer sample_count --with=range

# Strip all constraints
mdvs update example_kb reinfer status --with=none
```

`--with` takes a comma-separated list. Incompatible kinds (e.g., `range,categorical` on the same field) are rejected at parse time. `--with=none` cannot be combined with other kinds. `--with` requires named fields.

**Reinfer all fields:**

```bash
mdvs update example_kb reinfer
```

When no fields are specified, all `[[fields.field]]` entries are removed and rebuilt from the current files. Fields that no longer exist in any file are reported as removed.

All other config sections (`[scan]`, `[embedding_model]`, `[chunking]`, `[search]`, `[update]`) are preserved. This is the key difference from `init --force`, which rewrites the entire `mdvs.toml`.

## Output

### Compact (default)

When the schema is already up to date:

```
Scanned 43 files — no changes (37 unchanged) (dry run)
```

When new fields are discovered, they appear in an "Added" section with the same key-value format as [init](./init.md):

```
Scanned 44 files — 1 field(s) changed (37 unchanged) (dry run)

Added (1):
┌ category ────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 3 out of 44                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ (none)                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ projects/alpha/notes/**                           │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

When `reinfer` detects a type change, the "Changed" section shows old and new values with an arrow:

```
Scanned 43 files — 1 field(s) changed (36 unchanged)

Changed (1):
┌ drift_rate ──────────────┬───────────────────────────────────────────────────┐
│ type                     │ Float → String                                    │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

When a reinferred field no longer exists in any file:

```
Scanned 43 files — 1 field(s) changed (36 unchanged)

Removed (1):
┌ category ────────────────┬───────────────────────────────────────────────────┐
│ previously allowed       │ projects/alpha/notes/**                           │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result:

```
Read config: example_kb/mdvs.toml (2ms)
Scan: 44 files (3ms)
Infer: 38 field(s) (0ms)
Write config: example_kb/mdvs.toml (1ms)
Scanned 44 files — 1 field(s) changed (37 unchanged)

Added (1):
┌ category ────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
...
```

The field tables are identical in both modes — verbose only adds the step lines showing processing times.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success (changes written, or no changes needed) |
| `2` | Pipeline error (missing config, scan failure, build failure) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `field '<name>' is not in mdvs.toml` | `reinfer` names a field that doesn't exist |
| `--with requires named fields` | `--with` flag used without specifying fields |
| `--with: <X> and <Y> are mutually exclusive` | Incompatible constraint kinds in the same `--with` list |
| `--with=none cannot be combined with other kinds` | `none` mixed with other kinds in `--with` |
| `field name conflicts with internal column` | New field name collides with reserved names |
