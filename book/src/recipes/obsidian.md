# Obsidian

mdvs works well with [Obsidian](https://obsidian.md/) vaults — it can validate your YAML frontmatter for consistency and provide semantic search across all your notes. Everything runs locally, no external services needed.

## Setup

Point mdvs at your vault:

```bash
mdvs init path/to/vault
```

This scans all markdown files, infers a typed schema from your frontmatter, and writes `mdvs.toml`. If auto-build is enabled (the default), it also downloads the embedding model and builds the search index.

Two artifacts are created:

- **`mdvs.toml`** — commit this to version control
- **`.mdvs/`** — add to `.gitignore` (search index, can be rebuilt)

### .gitignore

mdvs respects `.gitignore` by default. If your vault has `.obsidian/` in `.gitignore` (many do), those files are automatically excluded from scanning. No extra configuration needed.

### .mdvsignore

For additional exclusions, create a `.mdvsignore` file at the vault root. It uses the same syntax as `.gitignore`:

```
# AI working directories
.claude/
.gemini/

# Template files (if using Templater)
_templates/

# Attachments (no frontmatter)
attachments/
assets/
```

Any directory that doesn't contain markdown with frontmatter is a good candidate for exclusion — it speeds up scanning and avoids noise in the schema.

## Common frontmatter patterns

Obsidian vaults typically use frontmatter like:

```yaml
---
title: My Note
tags: [project, research]
status: active
date: 2026-03-14
draft: false
---
```

mdvs infers types automatically:

| Field | Inferred type | Notes |
|---|---|---|
| `title` | String | |
| `tags` | String[] | Array of strings |
| `status` | String | |
| `date` | String | No Date type yet — dates are stored as strings |
| `draft` | Boolean | |

### Inconsistent types

If the same field has different types across notes (e.g., `priority` is an integer in some files and a string like `"high"` in others), mdvs widens to the broadest compatible type — usually String. See [Types & Widening](../concepts/types.md) for the full rules.

### Dataview fields

If you use the [Dataview](https://blacksmithgu.github.io/obsidian-dataview/) plugin, its inline fields (e.g., `key:: value`) are **not** picked up by mdvs — only YAML frontmatter between `---` fences is scanned. Dataview fields that appear in the YAML block are handled normally.

## Validation

Once `mdvs.toml` exists, use `check` to verify your frontmatter:

```bash
mdvs check path/to/vault
```

This catches:

- **Wrong types** — a Boolean field with a string value
- **Missing required fields** — a field that should be present in certain directories
- **Disallowed fields** — a field appearing where it shouldn't
- **Null violations** — null where it's not allowed

See [Validation](../concepts/validation.md) for the full rules.

### Tightening constraints

The inferred schema is permissive by default. To enforce stricter rules, edit `mdvs.toml` directly. For example, to require `tags` in all daily notes:

```toml
[[fields.field]]
name = "tags"
type = { array = "String" }
allowed = ["**"]
required = ["daily/**"]
nullable = false
```

### Updating the schema

When you introduce new frontmatter fields, run `update` to incorporate them:

```bash
mdvs update path/to/vault
```

This discovers new fields and adds them to `mdvs.toml` without touching existing field definitions. Use `--reinfer` to re-infer specific fields if you've reorganized your vault.

## Search

Build the index and search:

```bash
mdvs build path/to/vault
mdvs search "topic of interest" path/to/vault
```

Filter with `--where` on your frontmatter:

```bash
# Only active notes
mdvs search "topic" path/to/vault --where "status = 'active'"

# Notes with a specific tag
mdvs search "topic" path/to/vault --where "array_has(tags, 'research')"

# Notes in a specific directory
mdvs search "topic" path/to/vault --where "filepath LIKE 'projects/%'"
```

See the [Search Guide](../search-guide.md) for the full `--where` reference.

## Tips

- **Incremental builds** — only notes whose body changed since the last build are re-embedded. Frontmatter-only changes (updating tags, status) don't trigger re-embedding. Run `mdvs build` freely — it's fast when nothing changed.

- **Alongside Obsidian search** — mdvs search is semantic (finds conceptually related notes), while Obsidian's built-in search is keyword-based. They complement each other.

- **Large vaults** — mdvs has been tested on vaults with 500+ files and 2000+ chunks. A full build from scratch completes in under a second. Subsequent builds are incremental, re-embedding only changed files.

- **Ignore noisy fields** — if some frontmatter fields are auto-generated and you don't want to validate them, add them to the `ignore` list in `mdvs.toml`:
  ```toml
  [fields]
  ignore = ["cssclass", "kanban-plugin"]
  ```
