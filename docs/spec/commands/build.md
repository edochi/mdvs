# `mdvs build`

Check frontmatter, embed markdown, write the Lance index.

## Pipeline

`cmd/build/mod.rs` → `run()` → `pub async fn build_core()`. `build_core` is public so profiling examples (`crates/mdvs/examples/profile_pipeline.rs`) can drive it directly and read the per-phase `StepEntry` timings.

1. **Read config** — `MdvsToml::read()` + `validate()`. Fill missing build sections (`[embedding_model]`, `[chunking]`, `[search]`) with defaults or `--set-*` values.
2. **Auto-update** — if `[build].auto_update` is true (the default), runs the inference pass and writes any newly discovered fields back to `mdvs.toml`. Disable with `--no-update` for deterministic CI builds.
3. **Scan** — `ScannedFiles::scan(path, &config.scan)`
4. **Validate** — same as `check`: `validate::validate()` (frontmatter errors → field values → required fields → deterministic `collect_violations`). Aborts on any violation — no dirty data in index.
5. **Classify** — compare scanned files against `FileIndexEntry` projected from the existing Lance dataset. Returns `ClassifyData` with `removed_file_ids`, `needs_embedding`, `retained_chunks`, and the file_id map:
   - New (no previous entry) → chunk + embed
   - Edited (hash differs) → re-chunk + re-embed (keep file_id)
   - Unchanged (hash matches) → retain existing chunks
   - Removed (in index, not in scan) → drop
6. **Config change check** — compare current `BuildMetadata` against stored. Mismatch → error unless `--force`.
7. **Load model** — `Embedder::load()` (`index/embed.rs`). Skipped if `needs_embedding == 0`.
8. **Chunk + embed** — for each new/edited file: `Chunks::new(body, max_chars)` → `embedder.embed_batch(texts)` → `Vec<ChunkRow>`
9. **Merge** — combine retained chunks (from unchanged files) with new chunks
10. **Write** — `cmd::build::write::write_index_step` dispatches across three paths:
    - **Skip** when no files were removed AND no new chunks were produced AND it's not a full rebuild. Returns `WriteOutcome::Skipped`; the step appears as `Skipped` in the rendered output. The skip predicate uses `new_chunks_count` (not file count) because empty-body files like Hugo `_index.md` always classify as needing embedding but produce zero chunks.
    - **Full overwrite** when `full_rebuild` is true (first build or `--force`) — `LanceBackend::write_index()` recreates the table via `CreateTableMode::Overwrite` and rebuilds the FTS + (above 10k chunks) IVF-PQ indexes.
    - **Incremental** otherwise — `LanceBackend::write_index_incremental()` deletes the rows for `file_ids_to_clear` (= new + edited + removed file_ids), appends the freshly embedded chunks, refreshes the schema metadata via `NativeTable::replace_schema_metadata`, and runs `optimize(All)` so the FTS + vector indexes incorporate the delta without a full rebuild.

Returns `BuildOutcome` with file/chunk counts, `new_fields`, `file_details`.

## Key points

- **Build includes check** — validation runs before embedding. Any violation aborts the build.
- **Incremental by default** — `content_hash` (xxh3 on body) determines what needs re-embedding. Frontmatter-only changes don't trigger re-embedding (but rewrite the `data` column on every retained chunk row).
- **Model skip** — if all files are unchanged, model loading is skipped entirely (fast no-op).
- **Write skip** — on an unchanged corpus, the index write itself is skipped too (no Lance dataset rewrite, no `optimize`, no FTS rebuild). Skipped steps render as nothing in text output but appear in `--output json` step lists as `"status": "skipped"`.
- **`--force`** — required when config changes (model, chunk_size, prefix) are detected, or when the schema content hash differs from the stored hash. The schema-hash error reads: `"schema: fields, types, constraints, path-scoping, or preprocessors have changed"`. First build (no existing Lance dataset) never needs `--force`.
- **Schema hash** — `compute_schema_hash(config)` hashes the post-translation canonical JSON of `dsl_to_canonical(config)` via xxh3-64. Stored as `mdvs.schema_hash` on the Lance table metadata. Pre-Wave-B builds without this key read as `""` → always treated as changed.

See [storage.md](../storage.md) for the Lance dataset schema and incremental classification details.
