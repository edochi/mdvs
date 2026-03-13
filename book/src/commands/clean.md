# clean

Delete the search index.

## Usage

```bash
mdvs clean [path]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`clean` deletes the `.mdvs/` directory, which contains the Parquet files that make up the search index. The `mdvs.toml` configuration file is never touched — you can rebuild the index at any time with [build](./build.md).

The command is idempotent — running it when `.mdvs/` doesn't exist is a no-op. It also refuses to delete if `.mdvs/` is a symlink, as a safety measure.

## Output

### Compact (default)

```bash
mdvs clean example_kb
```

```
Cleaned "example_kb/.mdvs"
```

When there's nothing to clean:

```
Nothing to clean — "example_kb/.mdvs" does not exist
```

### Verbose (`-v`)

```
Delete index: "example_kb/.mdvs" (2 files, 113.6 KB)

Cleaned "example_kb/.mdvs"

2 files | 113.6 KB
```

Verbose output shows the file count and total size of the deleted directory.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Success (including when nothing to clean) |
| `2` | Pipeline error (symlink detected, I/O failure) |

## Errors

| Error | Cause |
|---|---|
| `.mdvs is a symlink` | Refuses to delete symlinks for safety — remove it manually |
