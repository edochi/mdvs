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

The output is organized into sections: Config, Index (if built), and one key-value table per field. Only a few fields are shown here:

```
43 files, 37 fields, 59 chunks

Config:
┌──────────────────────────┬───────────────────────────────────────────────────┐
│ scan glob                │ **                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ ignored fields           │ (none)                                            │
└──────────────────────────┴───────────────────────────────────────────────────┘

Index:
┌──────────────────────────┬───────────────────────────────────────────────────┐
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ revision                 │ none                                              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ chunk size               │ 1024                                              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ built                    │ 2026-03-29T15:22:21.347671+00:00                  │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ config                   │ match                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 43 out of 43                                      │
└──────────────────────────┴───────────────────────────────────────────────────┘

37 fields:
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
```

The `config` row shows `match` when `mdvs.toml` matches the index metadata, or `changed` when the config has been modified since the last build. The `files` row shows indexed files vs files on disk.

When no index has been built:

```
43 files, 37 fields

Config:
┌──────────────────────────┬───────────────────────────────────────────────────┐
│ scan glob                │ **                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ ignored fields           │ (none)                                            │
└──────────────────────────┴───────────────────────────────────────────────────┘

37 fields:
...
```

The Index section is omitted and the summary shows only files and fields (no chunk count).

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result:

```
Read config: example_kb/mdvs.toml (2ms)
Scan: 43 files (3ms)
Read index: 43 files, 59 chunks (2ms)
43 files, 37 fields, 59 chunks

Config:
...
```

The tables are identical in both modes — verbose only adds the step lines showing processing times.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success (including when no index exists) |
| `2` | Pipeline error (missing config, parquet read failure) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
