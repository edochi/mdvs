# `mdvs build`

Check frontmatter, embed markdown, write Parquet index.

## Pipeline

`cmd/build.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`. Fill missing build sections (`[embedding_model]`, `[chunking]`, `[search]`) with defaults or `--set-*` values.
2. **Auto-update** — if `[build].auto_update` is true, runs `update::run()` first
3. **Scan** — `ScannedFiles::scan(path, &config.scan)`
4. **Validate** — same as `check`: `check_field_values()` + `check_required_fields()`. Aborts on any violation — no dirty data in index.
5. **Classify** — compare scanned files against `FileIndexEntry` from existing `files.parquet`:
   - New (no previous entry) → chunk + embed
   - Edited (hash differs) → re-chunk + re-embed (keep file_id)
   - Unchanged (hash matches) → retain existing chunks
   - Removed (in index, not in scan) → drop
6. **Config change check** — compare current `BuildMetadata` against stored. Mismatch → error unless `--force`.
7. **Load model** — `Embedder::load()` (`index/embed.rs`). Skipped if `needs_embedding == 0`.
8. **Chunk + embed** — for each new/edited file: `Chunks::new(body, max_chars)` → `embedder.embed_batch(texts)` → `Vec<ChunkRow>`
9. **Merge** — combine retained chunks (from unchanged files) with new chunks
10. **Write** — `Backend::write_index()` → `files.parquet` + `chunks.parquet` with `BuildMetadata`

Returns `BuildOutcome` with file/chunk counts, `new_fields`, `file_details`.

## Key points

- **Build includes check** — validation runs before embedding. Any violation aborts the build.
- **Incremental by default** — `content_hash` (xxh3 on body) determines what needs re-embedding. Frontmatter-only changes don't trigger re-embedding.
- **Model skip** — if all files are unchanged, model loading is skipped entirely (fast no-op).
- **`--force`** — required when config changes (model, chunk_size, prefix) are detected. First build (no parquets) never needs `--force`.

See [storage.md](../storage.md) for Parquet schema and incremental classification details.
