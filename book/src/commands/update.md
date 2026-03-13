# update

Re-scan files, infer new fields, and update the schema.

## Usage

```bash
mdvs update [path] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--reinfer <field>` | | Re-infer a specific field (repeatable) |
| `--reinfer-all` | | Re-infer all fields from scratch |
| `--build` | from config | Override auto-build setting |
| `--dry-run` | | Preview changes without writing anything |

`--reinfer` and `--reinfer-all` cannot be used together.

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`update` re-scans the directory using the existing `[scan]` config, infers types and path patterns from the current files, and merges the results into `mdvs.toml`. Unlike [init](./init.md), it preserves all existing configuration — only the `[fields]` section changes.

### Default mode

By default, `update` only discovers **new** fields — fields that appear in frontmatter but aren't yet in `mdvs.toml` (either as `[[fields.field]]` entries or in the `ignore` list). Existing fields are protected: their types, allowed/required patterns, and nullable flags don't change.

Fields that disappear (no longer in any file) are kept in `mdvs.toml` by default. This is conservative — removing a field from the schema is an explicit action.

### `--reinfer`

Re-infer one or more specific fields. The named fields are removed from `mdvs.toml` and re-inferred from scratch, as if they'd never been seen. All other fields stay protected.

```bash
mdvs update example_kb --reinfer drift_rate --reinfer priority
```

Fails if a named field isn't in `mdvs.toml`.

### `--reinfer-all`

Re-infer every field from scratch. All `[[fields.field]]` entries are removed and rebuilt from the current files. Fields that no longer exist in any file are reported as removed.

All other config sections (`[scan]`, `[embedding_model]`, `[chunking]`, `[search]`, `[update]`) are preserved. This is the key difference from `init --force`, which rewrites the entire `mdvs.toml`.

### Auto-build

If `auto_build = true` in the `[update]` section (the default), `update` runs the full build pipeline after writing the updated config: validate, classify files, embed new/changed content, write index. The `--build` flag overrides the config setting.

Auto-build only triggers when there are actual field changes (added, changed, or removed fields). If the schema is already up to date, no build runs — even if new files were added. To pick up new files without schema changes, run [build](./build.md) directly.

## Output

### Compact (default)

When the schema is already up to date:

```
Scanned 43 files — no changes (dry run)
```

When new fields are discovered:

```
Scanned 44 files — 1 field(s) changed (dry run)

╭────────────────────────┬───────────────────┬───────────────────┬─────────────╮
│ "category"             │ added             │ String            │             │
╰────────────────────────┴───────────────────┴───────────────────┴─────────────╯
```

When `--reinfer` detects a type change:

```
Scanned 44 files — 2 field(s) changed (dry run)

╭────────────────────────┬───────────────────┬───────────────────┬─────────────╮
│ "category"             │ added             │ String            │             │
╰────────────────────────┴───────────────────┴───────────────────┴─────────────╯
╭───────────────────────────────────────────┬──────────────────────────────────╮
│ "drift_rate"                              │ type                             │
╰───────────────────────────────────────────┴──────────────────────────────────╯
```

When a reinferred field no longer exists:

```
Scanned 43 files — 1 field(s) changed (dry run)

╭────────────────────────────────────────┬─────────────────────────────────────╮
│ "category"                             │ removed                             │
╰────────────────────────────────────────┴─────────────────────────────────────╯
```

### Verbose (`-v`)

Added fields show the inferred path patterns:

```
Scanned 44 files — 1 field(s) changed (dry run)

╭─────────────────────────────┬───────────────────────┬────────────────────────╮
│ "category"                  │ added                 │ String                 │
├─────────────────────────────┴───────────────────────┴────────────────────────┤
│   found in:                                                                  │
│     - "projects/alpha/notes/**"                                              │
╰──────────────────────────────────────────────────────────────────────────────╯
```

Changed fields show old and new values for each aspect that differs:

```
╭────────────────────────┬──────────────────┬────────────────┬─────────────────╮
│ field                  │ aspect           │ old            │ new             │
│ "drift_rate"           │ type             │ Float          │ String          │
╰────────────────────────┴──────────────────┴────────────────┴─────────────────╯
```

Removed fields show where they were previously allowed:

```
╭──────────────────────────────┬───────────────────────────┬───────────────────╮
│ "category"                   │ removed                   │                   │
├──────────────────────────────┴───────────────────────────┴───────────────────┤
│   previously in:                                                             │
│     - "projects/**"                                                          │
╰──────────────────────────────────────────────────────────────────────────────╯
```

Verbose output also shows the pipeline steps before the result (Read config, Scan, Infer, Write config, etc.).

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success (changes written, or no changes needed) |
| `2` | Pipeline error (missing config, scan failure, build failure) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `field '<name>' is not in mdvs.toml` | `--reinfer` names a field that doesn't exist |
| `cannot use --reinfer and --reinfer-all together` | Conflicting flags |
| `field name conflicts with internal column` | New field name collides with reserved names |
