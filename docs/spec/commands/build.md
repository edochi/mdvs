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
6. If `--dry-run`: print build plan, return
7. Compare toml config against existing parquet metadata (see [Config changes](#config-changes))
8. Load model
9. Chunk, embed, write parquets to `.mdvs/`
10. Write build metadata to parquet key-value metadata
11. Print result

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

## Build strategy

**Current: full rebuild.** Every build re-chunks, re-embeds, and rewrites all parquets.

**Future (incremental):** use content hashes from `files.parquet` to detect changes:
- File content changed → re-chunk, re-embed that file only
- New files → chunk, embed, add
- Deleted files → remove from parquets
- Frontmatter changed (same text) → update `files.parquet` only, `chunks.parquet` unchanged
- Config changed (model, chunk_size, glob) → full rebuild
- Schema changed (new field, type change) → rewrite `files.parquet` only

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
    pub files_built: usize,
    pub chunks_created: usize,
    pub model: String,
    pub model_revision: Option<String>,
    pub new_fields: Vec<NewField>,     // see shared.md
    pub full_rebuild: bool,
    pub dry_run: bool,
}
```

### Human format

```
Built 12 files, 47 chunks
Model: minishlab/potion-base-8M

Built index in '/path/to/vault/.mdvs'
```

### Human format (new fields)

```
Built 12 files, 47 chunks
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
# Build index (full rebuild for now)
mdvs build

# Build a specific directory
mdvs build ~/notes

# Change model (requires --force)
mdvs build --set-model minishlab/potion-base-32M --force

# Change chunk size (requires --force)
mdvs build --set-chunk-size 512 --force

# Preview what would be built
mdvs build --dry-run

# First build after init --auto-build false
mdvs build
```
