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

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`build` creates (or updates) the search index in `.mdvs/`. The pipeline:

1. **Read config** — parse `mdvs.toml`. If `[embedding_model]`, `[chunking]`, or `[search]` sections are missing, they're added with defaults and written back.
2. **Scan** — walk the directory and extract frontmatter.
3. **Validate** — check frontmatter against the schema (same as [check](./check.md)). If violations are found, the build aborts.
4. **Classify** — compare scanned files against the existing index to determine what needs embedding.
5. **Load model** — download or load the cached embedding model. Skipped if nothing needs embedding.
6. **Embed** — chunk and embed new/edited files.
7. **Write index** — write `files.parquet` and `chunks.parquet` to `.mdvs/`.

See [Search & Indexing](../concepts/search.md) for details on chunking, embedding, and how the index is structured.

### Incremental builds

Build is incremental by default. It classifies each file by comparing its content hash against the existing index:

| Status | Condition | Action |
|---|---|---|
| **new** | file not in existing index | chunk + embed |
| **edited** | file in index, content changed | chunk + re-embed |
| **unchanged** | file in index, content matches | keep existing chunks |
| **removed** | file in index, no longer on disk | drop from index |

Content hash covers the **file body only** (after frontmatter extraction). Frontmatter-only changes don't trigger re-embedding — but `files.parquet` is always rewritten with fresh frontmatter from the current scan.

When nothing needs embedding, the model is never loaded.

### Config changes

`build` detects when the embedding configuration has changed since the last build by comparing `mdvs.toml` against metadata stored in the parquet files. If a mismatch is found, the build refuses to proceed unless you pass `--force`:

```
config changed since last build:
  model: 'minishlab/potion-base-8M' → 'minishlab/potion-base-32M'
Use --force to rebuild with new config
```

The `--set-model`, `--set-revision`, and `--set-chunk-size` flags update `mdvs.toml` and require `--force` (since they change the config and trigger a full re-embed). For example, to switch to a larger model:

```bash
mdvs build --set-model minishlab/potion-base-32M --force
```

`--set-revision` pins the model to a specific HuggingFace commit SHA, ensuring reproducible embeddings even if the model is updated upstream:

```bash
mdvs build --set-revision abc123def --force
```

The revision is stored in `mdvs.toml` under `[embedding_model].revision` and checked against the parquet metadata on subsequent builds. See [Embedding](../concepts/search.md#embedding) for the full list of available models.

On the first build (no existing `.mdvs/`), `--force` is never needed.

## Output

### Compact (default)

Incremental build with one new file:

```
Built index — 44 files, 60 chunks

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ embedded                 │ 1 file                  │ 1 chunk                 │
│ unchanged                │ 43 files                │ 59 chunks               │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

When nothing needs embedding:

```
Built index — 43 files, 59 chunks

╭──────────────────────────┬─────────────────────────┬─────────────────────────╮
│ unchanged                │ 43 files                │ 59 chunks               │
╰──────────────────────────┴─────────────────────────┴─────────────────────────╯
```

When violations are found, the build aborts:

```
Build aborted — 6 violation(s) found. Run `mdvs check` for details.
```

### Verbose (`-v`)

```
Read config: example_kb/mdvs.toml
Scan: 44 files
Validate: 44 files — no violations
Classify: 44 files (full rebuild)
Load model: "minishlab/potion-base-8M" (256d)
Embed: 44 files (60 chunks)
Write index: 44 files, 60 chunks

Built index — 44 files, 60 chunks (full rebuild)

╭─────────────────────────┬─────────────────────────┬──────────────────────────╮
│ embedded                │ 44 files                │ 60 chunks                │
├─────────────────────────┴─────────────────────────┴──────────────────────────┤
│   - "README.md" (7 chunks)                                                   │
│   - "blog/drafts/grant-ideas.md" (2 chunks)                                  │
│   - "blog/drafts/upcoming-talk.md" (1 chunk)                                 │
│   ...                                                                        │
│   - "scratch.md" (1 chunk)                                                   │
╰──────────────────────────────────────────────────────────────────────────────╯
```

Verbose output shows each pipeline step with its result, and expands embedded files with per-file chunk counts.

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
| `config changed since last build` | Config differs from parquet metadata — use `--force` |
| `--set-model requires --force` | Changing model triggers full re-embed |
| `--set-chunk-size requires --force` | Changing chunk size triggers full re-embed |
| `dimension mismatch` | Model produces different dimensions than existing index — run `mdvs clean` first (see [TODO-0098](https://github.com/edochi/mdvs)) |
