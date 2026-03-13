# info

Show config and index status.

## Usage

```bash
mdvs info [path]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`info` reads `mdvs.toml`, counts files on disk, and reads the index metadata from `.mdvs/` (if it exists). It displays the current schema and index status without modifying anything.

Use it to check which fields are configured, whether the index is up to date, or if the config has changed since the last build.

## Output

### Compact (default)

```bash
mdvs info example_kb
```

```
43 files, 37 fields, 59 chunks

╭──────────────────────────────┬───────────────────────────────────────────────╮
│ model:                       │ minishlab/potion-base-8M                      │
│ config:                      │ match                                         │
│ files:                       │ 43/43                                         │
╰──────────────────────────────┴───────────────────────────────────────────────╯

╭──────────────┬───────────────┬───────────────┬───────────────┬───────────────╮
│ "title"      │ String        │ required: "bl │ allowed: "blo │               │
│              │               │ og/**", ...   │ g/**", ...    │               │
│ "tags"       │ String[]      │ required: "bl │ allowed: "blo │               │
│              │               │ og/published/ │ g/**", ...    │               │
│              │               │ **", ...      │               │               │
│ "draft"      │ Boolean       │ required: "bl │ allowed: "blo │               │
│              │               │ og/**"        │ g/**"         │               │
│ "drift_rate" │ Float?        │ required: "pr │ allowed: "pro │               │
│              │               │ ojects/alpha/ │ jects/alpha/n │               │
│              │               │ notes/**"     │ otes/**"      │               │
│ ...          │               │               │               │               │
╰──────────────┴───────────────┴───────────────┴───────────────┴───────────────╯
```

The summary line shows files on disk, field count, and chunk count. The index block shows the embedding model, whether the config matches the index (`match` or `changed`), and how many files are indexed vs on disk. The field table lists every `[[fields.field]]` entry with its type, required patterns, and allowed patterns.

When no index has been built:

```
43 files, 37 fields
```

The index block is omitted and the summary shows only files and fields.

### Verbose (`-v`)

```
Read config: example_kb/mdvs.toml
Scan: 43 files
Read index: 43 files, 59 chunks

43 files, 37 fields, 59 chunks

╭────────────────────────────┬─────────────────────────────────────────────────╮
│ model:                     │ minishlab/potion-base-8M                        │
│ revision:                  │ none                                            │
│ chunk size:                │ 1024                                            │
│ built:                     │ 2026-03-13T22:46:02.902129+00:00                │
│ config:                    │ match                                           │
│ files:                     │ 43/43                                           │
╰────────────────────────────┴─────────────────────────────────────────────────╯

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
╭──────────────────────────────┬────────────────────────┬──────────────────────╮
│ "drift_rate"                 │ Float?                 │ 3/43                 │
├──────────────────────────────┴────────────────────────┴──────────────────────┤
│   required:                                                                  │
│     - "projects/alpha/notes/**"                                              │
│   allowed:                                                                   │
│     - "projects/alpha/notes/**"                                              │
│   nullable: true                                                             │
╰──────────────────────────────────────────────────────────────────────────────╯
...
```

Verbose output adds pipeline steps, the full index details (revision, chunk size, build timestamp), and expands each field into a record showing its glob patterns. The count column (e.g., `9/43`) shows how many scanned files contain the field.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success (including when no index exists) |
| `2` | Pipeline error (missing config, parquet read failure) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
