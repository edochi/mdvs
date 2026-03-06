# Configuration

All configuration lives in `mdvs.toml`, which `init` generates automatically. This page explains each section and how to customize it.

## Sections overview

| Section | Purpose | Required |
|---|---|---|
| `[scan]` | Which files to include | Yes |
| `[fields]` | Schema: field types, constraints | Yes |
| `[update]` | Behavior when re-scanning | Yes |
| `[embedding_model]` | Which model to use for embeddings | For search |
| `[chunking]` | How to split files into chunks | For search |
| `[search]` | Search defaults | For search |
| `[storage]` | Internal column prefix (advanced) | No |

The first three sections are always present. The rest are added when you build a search index (either via `init` or `build`).

## [scan]

Controls which files mdvs scans.

```toml
[scan]
glob = "**"
include_bare_files = false
skip_gitignore = false
```

| Field | Default | Description |
|---|---|---|
| `glob` | `"**"` | Glob pattern for matching markdown files |
| `include_bare_files` | `false` | Include files without frontmatter in validation |
| `skip_gitignore` | `false` | Ignore `.gitignore` rules when scanning |

The glob is relative to the directory containing `mdvs.toml`. Use `"blog/**"` to only scan a subdirectory.

## [fields]

Defines the schema — what frontmatter fields exist, their types, and constraints.

```toml
[fields]
ignore = ["internal_notes"]

[[fields.field]]
name = "title"
type = "String"
allowed = ["**"]
required = ["**"]

[[fields.field]]
name = "tags"
type = { array = "String" }
allowed = ["blog/**"]
required = ["blog/**"]

[[fields.field]]
name = "draft"
type = "Boolean"
allowed = ["blog/**"]
required = []
```

### Field properties

| Property | Default | Description |
|---|---|---|
| `name` | (required) | The frontmatter field name |
| `type` | `"String"` | The field type (see below) |
| `allowed` | `["**"]` | Glob patterns where this field may appear |
| `required` | `[]` | Glob patterns where this field must appear |

### Types

| Type | TOML syntax | Example value |
|---|---|---|
| String | `"String"` | `"hello"` |
| Boolean | `"Boolean"` | `true` |
| Integer | `"Integer"` | `42` |
| Float | `"Float"` | `3.14` |
| Array of T | `{ array = "String" }` | `["a", "b"]` |
| Object | `{ object = { name = "String", age = "Integer" } }` | `{ name: "...", age: 30 }` |

Type inference follows a widening rule: if a field is `Integer` in one file and `Float` in another, it becomes `Float`. If types are incompatible, the field widens to `String`.

### Ignore list

Fields in the `ignore` list are known to mdvs but not validated — no type checking, no allowed/required constraints. Use this for fields you want to acknowledge but not enforce:

```toml
[fields]
ignore = ["internal_notes", "wip"]
```

## [update]

Controls what happens when you run `mdvs update`.

```toml
[update]
auto_build = true
```

| Field | Default | Description |
|---|---|---|
| `auto_build` | `true` | Automatically rebuild the search index after updating fields |

## [embedding_model]

Which model to use for generating embeddings.

```toml
[embedding_model]
provider = "model2vec"
name = "minishlab/potion-base-8M"
revision = "main"
```

| Field | Default | Description |
|---|---|---|
| `provider` | `"model2vec"` | Embedding provider |
| `name` | `"minishlab/potion-base-8M"` | Model name (HuggingFace repo) |
| `revision` | (none) | Pin to a specific model revision |

The default model (`potion-base-8M`) is ~8MB and runs instantly on CPU. Any Model2Vec-compatible model from HuggingFace works.

Changing the model requires rebuilding the index with `mdvs build --force`.

## [chunking]

How files are split into chunks before embedding.

```toml
[chunking]
max_chunk_size = 1024
```

| Field | Default | Description |
|---|---|---|
| `max_chunk_size` | `1024` | Maximum characters per chunk |

mdvs uses semantic chunking — it splits on markdown structure (headings, paragraphs) rather than at arbitrary character boundaries. The `max_chunk_size` is an upper bound, not a target.

Changing the chunk size requires rebuilding with `mdvs build --force`.

## [search]

Default settings for the `search` command.

```toml
[search]
default_limit = 10
```

| Field | Default | Description |
|---|---|---|
| `default_limit` | `10` | Default number of results (overridden by `--limit`) |

## [storage]

Advanced: controls how internal columns are named in the Parquet files.

```toml
[storage]
internal_prefix = "_"
```

| Field | Default | Description |
|---|---|---|
| `internal_prefix` | `"_"` | Prefix for internal column names |

You only need this if a frontmatter field name collides with an internal column name (e.g., a field called `_file_id`). Change the prefix to avoid the collision.

This section is not written to `mdvs.toml` by default — add it manually if needed. Changing the prefix requires rebuilding with `mdvs build --force`.
