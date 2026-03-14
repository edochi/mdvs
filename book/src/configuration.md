# Configuration

All configuration lives in `mdvs.toml`, created by [init](./commands/init.md) and updated by [update](./commands/update.md). This page is a complete reference of every section and field.

## Sections overview

`mdvs.toml` has two groups of sections:

**Validation** (always present):
- [`[scan]`](#scan) — file discovery
- [`[update]`](#update) — update workflow
- [`[fields]`](#fields) — field definitions and ignore list

**Build** (optional — added by `init` with auto-build or by `build`):
- [`[embedding_model]`](#embedding_model) — model identity
- [`[chunking]`](#chunking) — chunk sizing
- [`[search]`](#search) — search defaults

There's also an optional [`[storage]`](#storage) section that's rarely needed.

When build sections are absent, validation commands ([check](./commands/check.md), [update](./commands/update.md)) work normally. The [build](./commands/build.md) command adds missing build sections with defaults on first run.

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

Controls the [update](./commands/update.md) command workflow.

```toml
[update]
auto_build = true
```

| Field | Type | Default | Description |
|---|---|---|---|
| `auto_build` | Boolean | `true` | Run build automatically after update infers field changes |

When `auto_build` is `true` and update finds changes, it triggers the full build pipeline (validate, embed, write index) after writing the updated config.

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

## `[search]`

Default settings for the [search](./commands/search.md) command.

```toml
[search]
default_limit = 10
```

| Field | Type | Default | Description |
|---|---|---|---|
| `default_limit` | Integer | `10` | Maximum results when `--limit` is not specified |

## `[storage]`

Internal storage settings. This section is rarely needed and omitted from `mdvs.toml` by default.

| Field | Type | Default | Description |
|---|---|---|---|
| `internal_prefix` | String | `"_"` | Prefix for internal parquet column names |

The search index stores your frontmatter fields alongside internal bookkeeping columns (`file_id`, `chunk_id`, `content_hash`, etc.) in the same Parquet files. To avoid name collisions, internal columns are prefixed — by default with `_`, producing names like `_file_id` and `_content_hash`.

This means frontmatter fields starting with `_` could collide with internal columns. If that happens, `init` and `update` will report an error. You can resolve it by choosing a different prefix:

```toml
[storage]
internal_prefix = "_mdvs_"
```

Changing the prefix requires `build --force`.

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

This is not the same as putting the field in the `ignore` list. Both prevent the field from being reported as new during `update`, but a `[[fields.field]]` entry tracks the field — it appears in `info` output with its type and patterns, and can be targeted by `update --reinfer`. The `ignore` list simply silences the field: no validation, no detail in `info`.

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

The invariant is `required ⊆ allowed` — you can't require a field in a path where it's not allowed. See [Schema Inference](./concepts/schema.md#path-patterns) for how these patterns are computed.

## Example

A representative subset from `example_kb/mdvs.toml` (37 fields total, 4 shown):

```toml
[scan]
glob = "**"
include_bare_files = true
skip_gitignore = false

[update]
auto_build = false

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
