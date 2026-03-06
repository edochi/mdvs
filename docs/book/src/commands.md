# Commands

Quick reference for all mdvs commands and their flags.

## mdvs init

Scan files, infer a schema, and optionally build a search index.

```bash
mdvs init [path] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory to initialize |
| `--model` | `minishlab/potion-base-8M` | Embedding model (HuggingFace repo) |
| `--revision` | (none) | Pin model to a specific revision |
| `--glob` | `**` | File matching pattern |
| `--include-bare-files` | `false` | Include files without frontmatter |
| `--chunk-size` | `1024` | Max characters per chunk |
| `--suppress-auto-build` | `false` | Only create schema, skip building |
| `--force` | `false` | Overwrite existing `mdvs.toml` |
| `--skip-gitignore` | `false` | Ignore `.gitignore` rules |

## mdvs check

Validate frontmatter against the schema. Read-only — never modifies files.

```bash
mdvs check [path]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |

Exit code 0 if valid, non-zero if violations found.

## mdvs update

Re-scan files and update field definitions in `mdvs.toml`.

```bash
mdvs update [path] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--reinfer` | (none) | Re-infer a specific field from scratch |
| `--reinfer-all` | `false` | Re-infer all fields from scratch |
| `--build` | (from config) | Override `auto_build` setting |

By default, only new fields are added. Existing fields keep their current type and constraints. Use `--reinfer` to reset a specific field, or `--reinfer-all` to re-infer everything.

## mdvs build

Validate frontmatter, then chunk, embed, and write the search index.

```bash
mdvs build [path] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--force` | `false` | Full rebuild (ignore incremental cache) |
| `--set-model` | (none) | Change embedding model |
| `--set-revision` | (none) | Change model revision |
| `--set-chunk-size` | (none) | Change chunk size |

Build always validates first — if `check` would fail, `build` aborts before embedding.

Config changes (`--set-*` flags or manual toml edits) are detected via Parquet metadata and require `--force`.

## mdvs search

Search the index with a natural language query.

```bash
mdvs search <query> [path] [flags]
```

| Flag | Default | Description |
|---|---|---|
| `query` | (required) | Natural language search query |
| `path` | `.` | Directory containing `mdvs.toml` |
| `--limit`, `-n` | (from config) | Maximum number of results |
| `--where` | (none) | SQL filter on frontmatter fields |

## mdvs info

Show configuration and index status.

```bash
mdvs info [path]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |

## mdvs clean

Delete the `.mdvs/` directory (search index). Does not touch `mdvs.toml`.

```bash
mdvs clean [path]
```

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |

## Global flags

These flags work with any command:

| Flag | Default | Description |
|---|---|---|
| `--output`, `-o` | `human` | Output format: `human` or `json` |
| `-v` | (off) | Verbose output (`-v` info, `-vv` debug, `-vvv` trace) |
