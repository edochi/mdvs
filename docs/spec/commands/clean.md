# `mdvs clean`

Delete the `.mdvs/` index directory.

## Pipeline

`cmd/clean.rs` → `run()`

1. **Check** — verify `.mdvs/` exists and is not a symlink
2. **Stats** — `walk_dir_stats()` counts files and sums sizes for the outcome
3. **Delete** — `fs::remove_dir_all()`

Returns `CleanOutcome` with `removed: bool`, `path`, `files_removed`, `size_bytes`.

## Key points

- **Destructive** — removes all parquet files and build metadata. Requires a `build` to recreate.
- **Config preserved** — `mdvs.toml` is not touched. Only `.mdvs/` is deleted.
- **No confirmation** — deletes immediately. No `--force` required.
