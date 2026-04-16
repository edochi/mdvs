---
name: mdvs
description: >-
  Semantic search and frontmatter validation for markdown directories.
  Use mdvs to find relevant notes by meaning (not just keywords), filter
  results by frontmatter fields with SQL, validate schema consistency,
  and detect field type or constraint violations. Use when the user asks
  to search notes or docs, validate frontmatter, set up a schema for
  markdown files, or when an mdvs.toml file exists in the project.
---

# mdvs — Markdown Validation & Search

A CLI that treats markdown directories as databases: schema inference, frontmatter validation, and semantic search with SQL filtering. Single binary, no external services.

## When to use which command

| User intent | Command |
|---|---|
| Set up a schema for a markdown directory | `mdvs init <path>` |
| Validate frontmatter against the schema | `mdvs check <path>` |
| Re-scan after adding/changing/removing files | `mdvs update <path>` |
| Re-infer a field's type or constraints | `mdvs update reinfer <field> <path>` |
| Build or rebuild the search index | `mdvs build <path>` |
| Search across notes | `mdvs search "<query>" <path>` |
| Check what's configured and indexed | `mdvs info <path>` |
| Delete the search index | `mdvs clean <path>` |

`<path>` defaults to `.` (current directory) for all commands.

## Two layers

mdvs has two independent layers:

1. **Validation** (`init`, `check`, `update`) — works immediately, no model download, no build step. Reads markdown files and validates frontmatter against `mdvs.toml`.
2. **Search** (`build`, `search`) — downloads an embedding model, chunks markdown content, and builds a local Parquet index in `.mdvs/`.

Validation stands alone. You never need to build an index just to validate.

## Key files

- **`mdvs.toml`** — the schema config, committed to version control. Source of truth for field types, allowed/required paths, and constraints. Created by `init`, updated by `update`.
- **`.mdvs/`** — build artifacts (Parquet files, cached model). Gitignored. Recreatable with `mdvs build`. Never edit directly.

## Command reference

### `mdvs init`

Scans markdown files, infers a typed schema from frontmatter, and writes `mdvs.toml`.

- `--force` — overwrite an existing `mdvs.toml` (deletes `.mdvs/` too)
- `--dry-run` — show what would be inferred without writing anything
- `--ignore-bare-files` — exclude files that have no frontmatter

Use `init --force` to start over from scratch. Use `update` to incrementally add new fields.

### `mdvs check`

Validates all frontmatter against the schema in `mdvs.toml`. Reports violations:

- **`MissingRequired`** — a required field is absent from a file
- **`WrongType`** — value doesn't match the declared type (e.g., string in an integer field)
- **`Disallowed`** — field appears in a file path not covered by its `allowed` globs
- **`InvalidCategory`** — value is not in the field's declared category list

New fields (present in files but not in `mdvs.toml`) are reported separately as informational — they don't cause a non-zero exit code. Run `update` to add them to the schema.

### `mdvs update`

Re-scans files and adds newly discovered fields to `mdvs.toml`. Does not remove or change existing fields by default.

- `mdvs update` — detect and add new fields only
- `mdvs update reinfer <field>` — re-infer type and constraints for a specific field
- `mdvs update reinfer <field> --dry-run` — preview what reinfer would change
- `mdvs update reinfer <field> --no-categorical` — reinfer without detecting categories
- `mdvs update reinfer <field> --categorical` — force categorical detection regardless of thresholds

Use `reinfer` when a field's type has changed (e.g., values evolved from integers to strings) or when you want to refresh its categorical constraints.

### `mdvs build`

Validates frontmatter (runs `check` internally), then chunks markdown content, generates embeddings, and writes Parquet files to `.mdvs/`.

- `--force` — full rebuild (ignore incremental cache)
- Incremental by default — only re-embeds new or edited files
- Aborts if `check` finds violations

The first build downloads the embedding model (~30 MB). Subsequent builds reuse the cached model.

### `mdvs search`

Semantic search across the indexed notes. Requires a built index (auto-builds if needed).

```bash
mdvs search "<query>" [path] [--where "<SQL>"] [--limit N] [-v]
```

- `--where` — SQL WHERE clause to filter on frontmatter fields
- `--limit` — max results (default: 10)
- `-v` — show best matching chunk text per result
- `--no-build` — skip auto-build, fail if no index exists
- `--no-update` — skip auto-update before building

#### `--where` filter examples

```bash
--where "draft = false"
--where "status = 'published'"
--where "priority = 'high' AND author = 'Alice'"
--where "sample_count > 20"
--where "tags = 'rust'"                            # checks if array contains value
--where "status IN ('draft', 'published')"
```

String values use single quotes. Field names with spaces or special characters need double-quote escaping: `--where "\"lab section\" = 'Photonics'"`.

### `mdvs info`

Shows the current config and index status: scan settings, field definitions, build metadata (model, chunk size, file counts). Use `-v` for full field detail.

### `mdvs clean`

Deletes the `.mdvs/` directory. Does not touch `mdvs.toml`.

## Output format

Use the default text output. It produces clean, readable tables and summaries. Only use `--output json` when you need to extract specific values programmatically (e.g., piping violation counts into another tool).

Use `-v` (verbose) to get detailed per-field or per-file information in both text and JSON modes.

## Exit codes

- **0** — success (no violations)
- **1** — violations found (`check` and `build`)
- **2** — error (bad config, missing files, model mismatch, etc.)

## Common workflows

### First-time setup

```bash
mdvs init          # scan, infer schema, write mdvs.toml
```

### Ongoing validation

```bash
mdvs check         # after editing files — are they still valid?
mdvs update        # after adding new frontmatter fields
mdvs check         # validate again after update
```

### Fixing a field that was inferred wrong

```bash
mdvs update reinfer priority --dry-run   # preview
mdvs update reinfer priority             # apply
```

### Searching with filters

```bash
mdvs search "calibration results"
mdvs search "setup" --where "draft = false"
mdvs search "budget" --where "status = 'active'" -v
mdvs search "experiment" --where "author = 'Giulia Ferretti'" --limit 5
```

### CI validation

```bash
mdvs check
# exit code 0 = all valid, 1 = violations found
```

## Things to know

- `build` always runs `check` first — if validation fails, the build aborts.
- `search` auto-runs `update` and `build` if needed (unless `--no-update` or `--no-build`).
- Field types are inferred automatically: `String`, `Integer`, `Float`, `Boolean`, `Array(T)`, `Object(...)`. Mixed types widen (e.g., `Integer` + `String` becomes `String`).
- Fields with low-cardinality repeated values are automatically detected as categorical (e.g., `status: draft/published/archived`). Out-of-category values are reported as `InvalidCategory` violations.
- `init --force` rewrites the entire config from scratch. `update` preserves existing config and only adds new fields. `update reinfer` re-infers specific fields.
- The model identity is tracked: if you change the model in `mdvs.toml`, `build` and `search` will require `--force` to confirm a full re-embed.
- `mdvs.toml` is the complete source of truth. There is no lock file.

## Examples

### Setting up a new vault from scratch

```bash
# User has a directory of markdown notes and wants to add schema validation
mdvs init ~/notes
# → Scans files, infers fields (title: String, tags: Array(String), draft: Boolean, ...),
#   writes ~/notes/mdvs.toml. Check runs automatically.

# Preview what init would infer without writing anything
mdvs init ~/notes --dry-run
```

### User added a new field to some files

The user started adding `category: tutorial` to blog posts. mdvs doesn't know about it yet.

```bash
mdvs check ~/notes
# → Reports "category" as a new field (informational, not a violation)

mdvs update ~/notes
# → Detects "category", adds it to mdvs.toml with inferred type, allowed/required paths

mdvs check ~/notes
# → Clean — category is now part of the schema
```

### Fixing violations after check

```bash
mdvs check ~/notes
# Output shows:
#   MissingRequired: "title" missing in blog/drafts/untitled.md
#   WrongType: "priority" expected Integer, got String in projects/alpha.md
#   InvalidCategory: "status" got "wip", expected one of [draft, published, archived]
```

For each violation type:
- **MissingRequired** — add the field to the file, or remove the path from `required` in `mdvs.toml`
- **WrongType** — fix the value in the file, or reinfer the field if the type should change: `mdvs update reinfer priority`
- **InvalidCategory** — fix the value in the file, or reinfer to update the category list: `mdvs update reinfer status`
- **Disallowed** — the field shouldn't be in that file path. Remove it from the file, or widen the `allowed` globs in `mdvs.toml`

### Searching with different filter patterns

```bash
# Find all notes about "machine learning" that are published
mdvs search "machine learning" --where "status = 'published'"

# Find high-priority items by a specific author
mdvs search "deadline" --where "priority = 'high' AND author = 'Alice'"

# Numeric comparison
mdvs search "experiment" --where "sample_count >= 100"

# Check if an array field contains a value
mdvs search "tutorial" --where "tags = 'beginner'"

# Multiple possible values
mdvs search "update" --where "status IN ('draft', 'review')"

# Verbose output — shows the matching text chunk from each result
mdvs search "calibration" -v
```

### Rebuilding after config changes

```bash
# User changed the model in mdvs.toml
mdvs build ~/notes
# → Error: model mismatch (config model differs from existing index)

mdvs build ~/notes --force
# → Full rebuild with the new model
```

### Working with subdirectories

mdvs schemas are directory-scoped. A large vault might have different schemas for different sections:

```bash
# Initialize just the blog section
mdvs init ~/notes/blog --glob "**"

# Initialize the whole vault
mdvs init ~/notes

# Search only within projects
mdvs search "budget" ~/notes/projects
```

### Edge cases

**Files without frontmatter (bare files):** By default, `init` includes bare files in the scan. They have no fields, which affects inference (e.g., a field can't be `required` for `**` if bare files exist). Use `--ignore-bare-files` to exclude them, or set `include_bare_files = false` in `[scan]`.

**Null values:** A field with `nullable = true` accepts null values. Null skips type and category checks. A field that is `required` but `nullable` passes if the key is present with a null value — it only fails if the key is entirely absent.

**Mixed-type fields:** If a field has integers in some files and strings in others, it widens to `String`. The integer values are stored as their string representation (e.g., `1` becomes `"1"`). This is not data loss — it's intentional widening.

**Field names with special characters:** Frontmatter keys with spaces, quotes, or other special characters work fine in `mdvs.toml` (TOML handles quoting). In `--where` clauses, wrap them in double quotes: `--where "\"author's note\" IS NOT NULL"`.

**Large vaults:** Scanning and validation are fast (reads all files every time, no incremental scan). Embedding is the expensive part — builds are incremental by default, only re-embedding new or changed files.

## Common errors

| Error | Cause | Fix |
|---|---|---|
| `mdvs.toml already exists` | Running `init` twice | Use `init --force` or `update` |
| `no markdown files found` | Wrong path or glob | Check the path and `[scan].glob` in config |
| `model mismatch` | Config model differs from index | Run `build --force` to re-embed |
| `field 'X' is not in mdvs.toml` | `reinfer` on unknown field | Check spelling, or run `update` first to add it |
| violations on `check` | Frontmatter doesn't match schema | Read the violation list, fix files or adjust schema |
