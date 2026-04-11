# Configuration

All configuration lives in `mdvs.toml`, created by [init](./commands/init.md) and updated by [update](./commands/update.md). This page is a complete reference of every section and field.

## Sections overview

`mdvs.toml` has two groups of sections:

**Validation** (always present):
- [`[scan]`](#scan) — file discovery
- [`[check]`](#check) — check command settings
- [`[fields]`](#fields) — field definitions and ignore list

**Build & search** (written by `init`, model/chunking filled by first `build`):
- [`[embedding_model]`](#embedding_model) — model identity
- [`[chunking]`](#chunking) — chunk sizing
- [`[build]`](#build) — build workflow settings
- [`[search]`](#search) — search defaults and auto-build/update

## Global flags

These flags apply to all commands:

| Flag | Values | Default | Description |
|---|---|---|---|
| `-o`, `--output` | `text`, `json` | `text` | Output format |
| `-v`, `--verbose` | | | Show detailed output (pipeline steps, expanded records) |
| `--logs` | `info`, `debug`, `trace` | (none) | Enable diagnostic logging to stderr |

---

## `[scan]`

Controls how markdown files are discovered.

```toml
[scan]
glob = "**"
include_bare_files = true
skip_gitignore = false
```

| Field | Type | Default | Description |
|---|---|---|---|
| `glob` | String | `"**"` | Glob pattern for matching markdown files |
| `include_bare_files` | Boolean | `true` | Include files without YAML frontmatter |
| `skip_gitignore` | Boolean | `false` | Don't read `.gitignore` patterns during scan |

When `include_bare_files` is `true`, files without frontmatter participate in inference (empty field set) and validation (can trigger `MissingRequired`). When `false`, they're excluded from the scan entirely.

## `[update]`

Placeholder for future update-specific settings. Currently empty — this section is hidden from `mdvs.toml` by default.

## `[check]`

Check command settings.

```toml
[check]
auto_update = true
```

| Field | Type | Default | Description |
|---|---|---|---|
| `auto_update` | Boolean | `false` | Auto-run update before validating |

When `auto_update` is `true`, `check` runs the update pipeline (scan, infer, write config) before validating. Set to `false` or use `--no-update` for deterministic CI validation against the committed `mdvs.toml`.

## `[embedding_model]`

Specifies the embedding model for semantic search. See [Embedding](./concepts/search.md#embedding) for available models.

```toml
[embedding_model]
provider = "model2vec"
name = "minishlab/potion-base-8M"
```

| Field | Type | Default | Description |
|---|---|---|---|
| `provider` | String | `"model2vec"` | Embedding provider (currently only `"model2vec"`) |
| `name` | String | `"minishlab/potion-base-8M"` | HuggingFace model ID |
| `revision` | String | (none) | Pin to a specific HuggingFace commit SHA for reproducibility |

The `provider` field can be omitted — it defaults to `"model2vec"`. The `revision` field only appears when explicitly set (e.g., via `build --set-revision`).

Changing the model or revision after a build requires `build --force` to re-embed all files.

## `[chunking]`

Controls semantic text splitting before embedding.

```toml
[chunking]
max_chunk_size = 1024
```

| Field | Type | Default | Description |
|---|---|---|---|
| `max_chunk_size` | Integer | `1024` | Maximum chunk size in characters |

The text splitter breaks each file's body into semantic chunks respecting markdown structure (headings, paragraphs, lists). Changing the chunk size after a build requires `build --force`.

## `[build]`

Build workflow settings.

```toml
[build]
auto_update = true
```

| Field | Type | Default | Description |
|---|---|---|---|
| `auto_update` | Boolean | `false` | Auto-run update before building |

When `auto_update` is `true`, `build` runs the update pipeline before building. Use `--no-update` to skip.

## `[search]`

Settings for the [search](./commands/search.md) command, including how internal columns are named in `--where` queries.

```toml
[search]
default_limit = 10
```

| Field | Type | Default | Description |
|---|---|---|---|
| `default_limit` | Integer | `10` | Maximum results when `--limit` is not specified |
| `internal_prefix` | String | `""` | Prefix for internal column names in `--where` queries |
| `aliases` | Map | `{}` | Per-column name overrides for internal columns |
| `auto_update` | Boolean | `false` | Auto-run update before building (when `auto_build` is true) |
| `auto_build` | Boolean | `false` | Auto-run build before searching |

### Internal column names

Beyond your frontmatter fields, the search index stores bookkeeping columns that mdvs uses internally. These *internal columns* are available in `--where` queries:

| Column | Contains |
|---|---|
| `filepath` | Relative file path (e.g., `blog/post.md`) |
| `file_id` | Unique identifier for each file |
| `content_hash` | Hash of the file body |
| `built_at` | Timestamp of last build |

By default, these are exposed with their raw names:

```bash
--where "filepath LIKE 'blog/%'"
```

If a frontmatter field name collides with an internal column (e.g., you have a field called `filepath`), search will error and suggest resolutions:

1. **Set a prefix** to namespace all internal columns:
   ```toml
   [search]
   internal_prefix = "_"
   ```
   Internal columns become `_filepath`, `_file_id`, etc.

2. **Set a per-column alias** to rename just the colliding column:
   ```toml
   [search.aliases]
   filepath = "path"
   ```
   The internal column becomes `path`, your frontmatter `filepath` stays bare.

3. **Rename the frontmatter field** in your markdown files.

Aliases take precedence over the prefix. See the [Search Guide](./search-guide.md) for full `--where` reference.

## `[fields]`

Defines field constraints and the ignore list. This is the largest section — it contains one `[[fields.field]]` entry per constrained field.

### Ignore list

```toml
[fields]
ignore = ["internal_id", "temp_notes"]
```

Fields in the `ignore` list are known but unconstrained — they skip all validation and are not reported as new fields by [check](./commands/check.md) or [update](./commands/update.md). A field cannot be in both `ignore` and `[[fields.field]]`.

### Field definitions

Each `[[fields.field]]` entry defines constraints on a frontmatter field:

```toml
[[fields.field]]
name = "title"
type = "String"
allowed = ["blog/**", "projects/**"]
required = ["blog/**", "projects/**"]
nullable = false
```

| Field | Type | Default | Description |
|---|---|---|---|
| `name` | String | (required) | Frontmatter key |
| `type` | FieldType | `"String"` | Expected value type |
| `allowed` | String[] | `["**"]` | Glob patterns where the field may appear |
| `required` | String[] | `[]` | Glob patterns where the field must be present |
| `nullable` | Boolean | `true` | Whether null values are accepted |
| `constraints` | Table | (absent) | Optional value constraints (see [Constraints](#constraints)) |

All fields except `name` have permissive defaults. A minimal entry with just a name:

```toml
[[fields.field]]
name = "title"
```

is equivalent to:

```toml
[[fields.field]]
name = "title"
type = "String"
allowed = ["**"]
required = []
nullable = true
```

This is not the same as putting the field in the `ignore` list. Both prevent the field from being reported as new during `update`, but a `[[fields.field]]` entry tracks the field — it appears in `info` output with its type and patterns, and can be targeted by `update reinfer`. The `ignore` list simply silences the field: no validation, no detail in `info`.

### Type syntax

Scalar types are plain strings:

```toml
type = "String"    # also: "Boolean", "Integer", "Float"
```

Arrays use an inline table:

```toml
type = { array = "String" }
```

Objects use a nested inline table:

```toml
type = { object = { author = "String", count = "Integer" } }
```

See [Types](./concepts/types.md) for the full type system, including widening rules.

### Path patterns

`allowed` and `required` are lists of glob patterns matched against relative file paths:

```toml
allowed = ["blog/**", "projects/alpha/**"]
required = ["blog/published/**"]
```

Patterns must end with `/*` (direct children) or `/**` (full subtree), or be exactly `*` or `**`. Bare paths like `blog` or file names like `blog/post.md` are not valid.

The invariant `required ⊆ allowed` is enforced — every required glob must be covered by some allowed glob. For example, `allowed = ["meetings/**"]` covers `required = ["meetings/all-hands/**"]` because any path matching the required pattern also matches the allowed one.

See [Schema Inference](./concepts/schema.md#path-patterns) for how these patterns are computed.

### Constraints

The optional `[fields.field.constraints]` sub-table adds value constraints beyond type checking. Currently, the `categories` key restricts values to an enumerated set:

```toml
[[fields.field]]
name = "status"
type = "String"

[fields.field.constraints]
categories = ["active", "archived", "completed", "draft", "published"]
```

Categories are auto-inferred during `init` and `update reinfer`. See [Constraints](./concepts/constraints.md) for the full reference.

### Inference thresholds

Two optional fields in `[fields]` control categorical auto-inference:

```toml
[fields]
max_categories = 10
min_category_repetition = 2
```

| Field | Type | Default | Description |
|---|---|---|---|
| `max_categories` | Integer | `10` | Max distinct values for a field to be inferred as categorical |
| `min_category_repetition` | Integer | `2` | Min average repetition (occurrences / distinct) for categorical inference |

These are hidden from `mdvs.toml` when set to their defaults. They only affect auto-inference — manually written `categories` are unaffected.

## Example

A representative subset from `example_kb/mdvs.toml` (37 fields total, 4 shown):

```toml
[scan]
glob = "**"
include_bare_files = true
skip_gitignore = false

[embedding_model]
provider = "model2vec"
name = "minishlab/potion-base-8M"

[chunking]
max_chunk_size = 1024

[search]
default_limit = 10

[fields]
ignore = []

[[fields.field]]
name = "title"
type = "String"
allowed = ["blog/**", "meetings/**", "people/**", "projects/**", "reference/protocols/**"]
required = ["blog/**", "meetings/**", "people/**", "projects/**", "reference/protocols/**"]
nullable = false

[[fields.field]]
name = "tags"
allowed = ["blog/**", "projects/alpha/*", "projects/alpha/notes/**", "projects/archived/**", "projects/beta/*", "projects/beta/notes/**"]
required = ["blog/published/**", "projects/alpha/notes/**", "projects/archived/**", "projects/beta/notes/**"]
nullable = false
type = { array = "String" }

[[fields.field]]
name = "drift_rate"
type = "Float"
allowed = ["projects/alpha/notes/**"]
required = ["projects/alpha/notes/**"]
nullable = true

[[fields.field]]
name = "calibration"
allowed = ["projects/alpha/notes/**"]
required = []
nullable = false
type = { object = { adjusted = { object = { intensity = "Float", wavelength = "Float" } }, baseline = { object = { intensity = "Float", notes = "String", wavelength = "Float" } } } }
```
