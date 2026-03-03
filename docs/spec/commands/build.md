# `mdvs build`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md), [check](check.md)

---

## Synopsis

```
mdvs build [path] [flags]
```

| Flag               | Type       | Default                    | Description                                                |
|--------------------|------------|----------------------------|------------------------------------------------------------|
| `path`             | positional | `.`                        | Directory containing mdvs.toml                             |
| `--set-model`      | string     | (none)                     | Change embedding model (writes to toml, requires `--force`) |
| `--set-revision`   | string     | (none)                     | Change model revision (writes to toml, requires `--force`) |
| `--set-chunk-size` | usize      | (none)                     | Change max chunk size (writes to toml, requires `--force`) |
| `--force`          | bool       | false                      | Confirm config changes that require full re-embed          |
| `--dry-run`        | bool       | false                      | Show what would be built, write nothing                    |

---

## Behavior

1. Read `mdvs.toml` (see [Prerequisites](check.md#prerequisites))
2. If build sections are missing from toml (`[embedding_model]`, `[chunking]`, `[search]`):
   - Add them with defaults (or from `--set-*` flag values)
   - Write updated toml
3. If `--set-*` flags provided on existing sections:
   - Require `--force` (see [Config changes](#config-changes))
4. Scan markdown files using `[scan]` config
5. Run check: validate frontmatter against `[fields]` rules
   - If violations: abort, report violations (same format as `check` command)
   - If new fields: collect for output (informational, does not abort)
6. Compare toml config against existing parquet metadata (see [Config changes](#config-changes))
7. If `--force` or config changed with `--force`: full rebuild (step 10)
8. Read existing index and classify files (see [Incremental build](#incremental-build))
9. If no files need embedding: update `files.parquet` with fresh frontmatter, print result, return
10. Load model
11. Chunk and embed new/edited files only (or all files on full rebuild)
12. Write parquets to `.mdvs/`:
    - `files.parquet`: all current files with fresh frontmatter from scan (removed files excluded)
    - `chunks.parquet`: existing chunks for unchanged files + new chunks for new/edited files (chunks for removed/edited files excluded)
13. Write build metadata to parquet key-value metadata
14. Print result

Progress messages ("Loading model...", "Embedding...") go to **stderr**.
The formatted result goes to **stdout**.

---

## Config changes

Build compares the current toml config against the existing parquet metadata (if parquets exist). If they differ, `--force` is required.

| Change                  | Detection                                      | Requires `--force` |
|-------------------------|-------------------------------------------------|---------------------|
| Model name changed      | toml `[embedding_model].name` vs parquet metadata | yes               |
| Model revision changed  | toml `[embedding_model].revision` vs parquet metadata | yes           |
| Chunk size changed      | toml `[chunking].max_chunk_size` vs parquet metadata | yes            |
| Scan glob changed       | toml `[scan].glob` vs parquet metadata          | yes                 |
| Field schema changed    | toml `[fields]` vs `data` column Arrow schema   | no — schema-only rewrite |
| First build (no parquets) | `.mdvs/` doesn't exist                        | no                  |

Without `--force`, config changes produce an error:

```
model changed from 'minishlab/potion-base-8M' to 'minishlab/potion-base-32M'
this requires a full re-embed — use --force to confirm
```

The `--set-*` flags update the toml value only when `--force` is also provided.

---

## Incremental build

Build is incremental by default. It uses `content_hash` from the existing `files.parquet` to detect what changed. The content hash covers only the file body (after frontmatter extraction), not the frontmatter itself.

### File classification

For each scanned file, compare against the existing index:

| Classification | Condition | Action |
|---|---|---|
| **New** | filename not in existing index | chunk, embed |
| **Edited** | filename in index, content_hash differs | chunk, re-embed (keep same file_id) |
| **Unchanged** | filename in index, content_hash matches | keep existing chunks |
| **Removed** | filename in index, not in scan | drop from both parquets |

### What gets written

- `files.parquet` is always fully rewritten from the fresh scan. Every file gets a FileRow built from the current frontmatter, regardless of classification. This ensures frontmatter-only changes (e.g., adding a tag) are captured without re-embedding.
- `chunks.parquet` is rewritten with: existing chunks for unchanged files + new chunks for new/edited files. Chunks for removed and edited files are dropped.

### Model loading

The embedding model is only loaded if there are new or edited files. If all files are unchanged (only frontmatter or removed files changed), the model is not loaded and no embedding runs.

### Full rebuild triggers

A full rebuild (re-embed everything) happens when:
- `--force` is provided
- Config changed with `--force` (model, chunk_size)
- First build (no existing parquets)

### Schema changes

If the Arrow schema changed (new field in toml, type change), `files.parquet` is rewritten with the new schema. This does not trigger re-embedding — only a schema-level rewrite of file rows.

---

## Parquet metadata

Stored as native key-value metadata in parquet files (not a separate file).

| Key              | Value                           |
|------------------|---------------------------------|
| `mdvs.model`     | Model name                      |
| `mdvs.revision`  | Model revision (or empty)       |
| `mdvs.chunk_size`| Max chunk size                  |
| `mdvs.glob`      | Scan glob pattern               |
| `mdvs.built_at`  | Build timestamp (ISO 8601)      |

Field schema is not stored in key-value metadata — it is the Arrow schema of the `data` Struct column in `files.parquet`.

### String field storage

When a field is typed as `String` (the widening top type), non-string values are serialized to their JSON representation:

| YAML value         | Stored as             |
|--------------------|-----------------------|
| `"hello"`          | `hello`               |
| `["a", "b"]`       | `["a","b"]`           |
| `{k: v}`           | `{"k":"v"}`           |
| `true`             | `true`                |
| `42`               | `42`                  |

No data is ever dropped — values that don't match the declared type are converted, not discarded.

---

## Output

```rust
pub struct BuildResult {
    pub files_total: usize,
    pub files_embedded: usize,       // new + edited (actually embedded)
    pub files_unchanged: usize,
    pub files_removed: usize,
    pub chunks_total: usize,
    pub model: String,
    pub model_revision: Option<String>,
    pub new_fields: Vec<NewField>,     // see shared.md
    pub full_rebuild: bool,
    pub dry_run: bool,
}
```

### Human format (full rebuild)

```
Built 12 files, 47 chunks (full rebuild)
Model: minishlab/potion-base-8M

Built index in '/path/to/vault/.mdvs'
```

### Human format (incremental, changes found)

```
Built 12 files, 47 chunks (2 new, 1 edited, 9 unchanged, 1 removed)
Model: minishlab/potion-base-8M

Built index in '/path/to/vault/.mdvs'
```

### Human format (incremental, no embedding needed)

```
Built 12 files, 47 chunks (no embedding needed)

Built index in '/path/to/vault/.mdvs'
```

### Human format (new fields)

```
Built 12 files, 47 chunks (3 new, 9 unchanged)
Model: minishlab/potion-base-8M

New fields (not in mdvs.toml):
  category (2 files)
  author (1 file)
Run 'mdvs update' to incorporate new fields.

Built index in '/path/to/vault/.mdvs'
```

### Dry run

```
Would build 12 files (~47 chunks)
Model: minishlab/potion-base-8M

(dry run, nothing written)
```

---

## Errors

| Condition                               | Message                                                                |
|-----------------------------------------|------------------------------------------------------------------------|
| Violations found                        | violations report (same as `check`), then `build aborted`              |
| Config changed without `--force`        | `<detail> — use --force to confirm full re-embed`                      |
| `--set-*` without `--force`             | `--set-model has no effect without --force`                            |
| `--set-model` and `--set-revision` with no existing parquets | (no error — first build, just use the values) |
| Model download failed                   | `failed to download model '<name>': <detail>`                          |

See also [Prerequisites](check.md#prerequisites) for toml validation errors.

---

## Examples

```bash
# Incremental build (only embed new/changed files)
mdvs build

# Build a specific directory
mdvs build ~/notes

# Force full rebuild (re-embed everything)
mdvs build --force

# Change model (requires --force, triggers full re-embed)
mdvs build --set-model minishlab/potion-base-32M --force

# Change chunk size (requires --force)
mdvs build --set-chunk-size 512 --force

# Preview what would be built
mdvs build --dry-run

# First build after init --auto-build false
mdvs build
```
