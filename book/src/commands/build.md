# build

Validate, embed, and write the search index.

## Usage

```bash
mdvs build [path] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--set-model` | | Change [embedding model](../concepts/search.md#embedding) (requires `--force`) |
| `--set-revision` | | Pin model to a specific HuggingFace revision (requires `--force`) |
| `--set-chunk-size` | | Change max chunk size in characters (requires `--force`) |
| `--force` | | Confirm config changes or trigger a full rebuild |
| `--no-update` | | Skip auto-update before building |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`build` creates (or updates) the search index in `.mdvs/`. The pipeline:

1. **Read config** — parse `mdvs.toml`. If `[embedding_model]`, `[chunking]`, or `[search]` sections are missing, they're added with defaults and written back.

By default, `build` auto-updates the schema before building (see [`[build].auto_update`](../configuration.md#build)). Use `--no-update` to validate against the committed schema (deterministic CI). The auto chain is cheap on unchanged corpora — no model load, no Lance write.

2. **Scan** — walk the directory and extract frontmatter.
3. **Validate** — check frontmatter against the schema (same as [check](./check.md)). If violations are found, the build aborts.
4. **Classify** — compare scanned files against the existing index to determine what needs embedding, what to retain, and what to drop.
5. **Load model** — download or load the cached embedding model. Skipped if nothing needs embedding.
6. **Embed** — chunk and embed new/edited files.
7. **Write index** — branches on the change set: **skip** (nothing changed and not a full rebuild), **full overwrite** (first build or `--force`), or **incremental** (delete the rows for new/edited/removed files, append the new chunks, refresh metadata, optimize). The Lance dataset is always at `.mdvs/index.lance/` with one row per chunk; a full-text BM25 index on `chunk_text` is rebuilt with the table; a cosine IVF-PQ vector index on `embedding` is created only above 10,000 chunks (smaller vaults rely on LanceDB's exact flat scan).

See [Search & Indexing](../concepts/search.md) for details on chunking, embedding, and how the index is structured.

### Incremental builds

Build is incremental by default. It classifies each file by comparing its content hash against the existing index:

| Status | Condition | Action |
|---|---|---|
| **new** | file not in existing index | chunk + embed |
| **edited** | file in index, content changed | chunk + re-embed |
| **unchanged** | file in index, content matches | keep existing chunks |
| **removed** | file in index, no longer on disk | drop from index |

Content hash covers the **file body only** (after frontmatter extraction). Frontmatter-only changes don't trigger re-embedding — but every chunk row is rewritten with fresh frontmatter from the current scan.

When nothing needs embedding, the model is never loaded. When the change set is empty (no new/edited/removed files), the index write itself is also skipped — `mdvs build` on an unchanged corpus does no Lance work at all.

### Config changes

`build` detects when the embedding configuration has changed since the last build by comparing `mdvs.toml` against metadata stored on the Lance dataset. If a mismatch is found, the build refuses to proceed unless you pass `--force`:

```
config changed since last build:
  model: 'minishlab/potion-base-8M' → 'minishlab/potion-base-32M'
Use --force to rebuild with new config
```

The same check covers **schema changes**. A hash of the post-translation JSON Schema is stored on the Lance dataset; if the current schema doesn't match, the build refuses with:

```
schema: fields, types, constraints, path-scoping, or preprocessors have changed
Use --force to rebuild with new schema
```

This catches edits to `[[fields.field]]` definitions, constraint changes, preprocessor changes, and path-scoping changes — anything that affects what gets stored in the `data` column of the index.

The `--set-model`, `--set-revision`, and `--set-chunk-size` flags update `mdvs.toml` and require `--force` (since they change the config and trigger a full re-embed). For example, to switch to a larger model:

```bash
mdvs build --set-model minishlab/potion-base-32M --force
```

`--set-revision` pins the model to a specific HuggingFace commit SHA, ensuring reproducible embeddings even if the model is updated upstream:

```bash
mdvs build --set-revision abc123def --force
```

The revision is stored in `mdvs.toml` under `[embedding_model].revision` and checked against the Lance dataset metadata on subsequent builds. See [Embedding](../concepts/search.md#embedding) for the full list of available models.

On the first build (no existing `.mdvs/`), `--force` is never needed.

## Output

### Compact (default)

When nothing needs embedding (incremental build, all files unchanged):

```
Built index — 43 files, 59 chunks

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ full rebuild             │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files total              │ 43                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files embedded           │ 0                                                 │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files unchanged          │ 43                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files removed            │ 0                                                 │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ chunks total             │ 59                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ chunks embedded          │ 0                                                 │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ chunks unchanged         │ 59                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ chunks removed           │ 0                                                 │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ new fields               │ (none)                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ embedded files           │ (none)                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ removed files            │ (none)                                            │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

When violations are found, the build aborts:

```
Build aborted — 6 violation(s) found. Run `mdvs check` for details.
```

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result. Steps that didn't need to run (model load on an unchanged corpus, the index write itself when nothing changed) are silently elided from the text output, but appear as `"status": "skipped"` in `--output json`. A full-rebuild verbose run:

```
Read config: example_kb/mdvs.toml (4ms)
Scan: 43 files (4ms)
Infer: 37 field(s) (0ms)
Validate: 43 files — no violations (87ms)
Classify: 43 to embed, 0 unchanged, 0 removed (0ms)
Load model: minishlab/potion-base-8M (24ms)
Embed: 43 files, 59 chunks (12ms)
Write index: 43 files, 59 chunks (1ms)
Built index — 43 files, 59 chunks (full rebuild)

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ full rebuild             │ true                                              │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files total              │ 43                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files embedded           │ 43                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files unchanged          │ 0                                                 │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ ...                                                                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ embedded files           │ README.md (7 chunks)                              │
│                          │ blog/drafts/grant-ideas.md (2 chunks)             │
│                          │ ...                                               │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ removed files            │ (none)                                            │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

The key-value table is identical in both modes — verbose only adds the step lines showing processing times. When files are embedded, the `embedded files` row lists each file with its chunk count.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Build completed successfully |
| `1` | Violations found — build aborted |
| `2` | Pipeline error (missing config, scan failure, config mismatch, model failure) |

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `config changed since last build` | Config differs from Lance dataset metadata — use `--force` |
| `--set-model requires --force` | Changing model triggers full re-embed |
| `--set-chunk-size requires --force` | Changing chunk size triggers full re-embed |
| `dimension mismatch` | Model produces different dimensions than existing index (incremental build only — `--force` bypasses this) |
