# Getting Started

This guide walks you through setting up mdvs on a directory of markdown files — from installation to your first search query.

## Install

With Cargo (requires the [Rust toolchain](https://rustup.rs/)):

```bash
cargo install mdvs
```

Or build from source:

```bash
git clone https://github.com/edochi/mdvs.git
cd mdvs
cargo install --path .
```

## Prepare your files

mdvs works with any directory of markdown files that use YAML frontmatter. For example, imagine a `notes/` directory:

```
notes/
├── blog/
│   ├── rust-errors.md
│   ├── async-patterns.md
│   └── draft-post.md
├── recipes/
│   ├── pasta.md
│   └── bread.md
└── ideas.md
```

Where each file has frontmatter like:

```yaml
---
title: Rust Error Handling
tags:
  - rust
  - errors
draft: false
---

# Rust Error Handling

Rust's error handling is built around the Result type...
```

## Initialize

Run `mdvs init` to scan your files, infer a schema, and build a search index:

```bash
mdvs init notes/
```

mdvs will:

1. **Scan** all markdown files matching the glob pattern (default: `**`)
2. **Infer** a typed schema from frontmatter — field names, types, where they appear, which are required
3. **Write** the schema to `notes/mdvs.toml`
4. **Build** a search index in `notes/.mdvs/`

You'll see output like:

```
Discovered 3 fields across 6 files
  title    String   (required in ["**"])
  tags     String[] (required in ["blog/**"])
  draft    Boolean  (allowed in ["blog/**"])

Built index: 6 files, 14 chunks
```

That's it. Your directory now has a schema and a search index.

## Search

Find files by meaning:

```bash
mdvs search "error handling patterns" notes/
```

```
0.72  blog/rust-errors.md
0.58  blog/async-patterns.md
0.31  ideas.md
```

The score (0–1) is cosine similarity — higher means more relevant. Results are ranked by the best-matching chunk in each file.

## Filter with SQL

Use `--where` to filter on frontmatter fields:

```bash
mdvs search "rust" --where "draft = false" notes/
```

Only files where `draft` is `false` are returned. You can use any SQL expression:

```bash
mdvs search "cooking" --where "tags IS NOT NULL" --limit 3 notes/
```

## Validate

Check your files against the inferred schema:

```bash
mdvs check notes/
```

If everything is valid:

```
Checked 6 files — no violations
```

If there are problems:

```
blog/draft-post.md: missing required field 'title'
recipes/pasta.md: field 'draft' not allowed in 'recipes/**'

Checked 6 files — 2 violations
```

## What's in mdvs.toml?

After `init`, open `notes/mdvs.toml`. It looks like this:

```toml
[scan]
glob = "**"
include_bare_files = false

[embedding_model]
provider = "model2vec"
name = "minishlab/potion-base-8M"

[chunking]
max_chunk_size = 1024

[update]
auto_build = true

[search]
default_limit = 10

[fields]
ignore = []

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

This is your schema. You can edit it — tighten constraints, change allowed paths, mark fields as required. Then run `mdvs check` to enforce it.

## Keep it up to date

As you add files with new frontmatter fields, run:

```bash
mdvs update notes/
```

mdvs re-scans your files, discovers new fields, and adds them to `mdvs.toml`. Existing fields are preserved. If `auto_build` is enabled (the default), the search index is rebuilt automatically.

## Next steps

- [Configuration](./configuration.md) — customize the schema, change the model, tune chunking
- [Searching](./searching.md) — SQL filtering, JSON output, scripting
- [Commands](./commands.md) — full reference for all commands and flags
