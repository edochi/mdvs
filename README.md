# mdvs — Markdown Validation & Search

<div align="center">

[![CI](https://github.com/edochi/mdvs/actions/workflows/ci.yml/badge.svg)](https://github.com/edochi/mdvs/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/mdvs)](https://crates.io/crates/mdvs)
[![downloads](https://img.shields.io/crates/d/mdvs)](https://crates.io/crates/mdvs)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-2021-orange.svg)](https://www.rust-lang.org/)
[![Docs](https://img.shields.io/badge/docs-mdBook-green.svg)](https://edochi.github.io/mdvs/)

</div>

mdvs infers a typed schema from your markdown frontmatter, validates it, and lets you search everything with natural language + SQL. Single binary, runs locally, no setup.

<div align="center">

  :x: A Document Database

  :white_check_mark: A Database for Documents

</div>

Schema inference, frontmatter validation, and semantic search for markdown directories. For Obsidian vaults, Zettelkasten, docs-as-code, research notes, wikis — any directory of markdown files with (or without) frontmatter.

## Why mdvs?

Markdown files can have a YAML block at the top called **frontmatter** — structured fields that describe the document:

```markdown
---
title: Rust Tips
tags: [rust, programming]
draft: false
---

# Rust Tips

Your content here...
```

`title`, `tags`, and `draft` are frontmatter fields. Most tools treat these as flat text or ignore them entirely. mdvs sees structure — your directories, your fields, your types. It infers which fields belong in which directories, validates that they're consistent, and lets you search everything with natural language and SQL.

No config to write. No schema to define. Point it at a directory and it figures it out.

## Install

### Prebuilt binary (macOS / Linux)

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/edochi/mdvs/releases/latest/download/mdvs-installer.sh | sh
```

### From crates.io

```bash
cargo install mdvs
```

### From source

```bash
git clone https://github.com/edochi/mdvs.git
cd mdvs
cargo install --path .
```

## How it works

mdvs treats your markdown directory as a database — and your directory structure as part of the schema.

Consider a simple knowledge base:

```
notes/
├── blog/
│   ├── rust-tips.md        ← title, tags, draft
│   └── half-baked-idea.md  ← title, draft
├── team/
│   ├── alice.md            ← title, role, email
│   └── bob.md              ← title, role
└── meetings/
    └── weekly.md           ← title, date, attendees
```

Different directories, different fields. mdvs sees this.

### Infer

```bash
mdvs init notes/
```

mdvs scans every file, extracts frontmatter, and infers which fields belong where:

```
Initialized 5 files — 7 field(s)

┌ title ───────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 5 out of 5                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ **                                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ **                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ draft ───────────────────┬───────────────────────────────────────────────────┐
│ type                     │ Boolean                                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 2 out of 5                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ (none)                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ blog/**                                           │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ role ────────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 2 out of 5                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ team/**                                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ team/**                                           │
└──────────────────────────┴───────────────────────────────────────────────────┘

...
```

`draft` belongs in `blog/`. `role` belongs in `team/`. The directory structure is the schema.

### Validate

Two new files appear — both without `role`:

```
notes/
├── blog/
│   └── new-post.md    ← title, draft  (no role)
├── team/
│   └── charlie.md     ← title         (no role)
└── ...
```

```bash
mdvs check notes/
```

```
Checked 7 files — 1 violation(s)

Violations (1):
┌ role ────────────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Missing required                                  │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ required in ["team/**"]                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ team/charlie.md                                   │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

`charlie.md` is missing `role` — but `new-post.md` isn't flagged. mdvs knows `role` belongs in `team/`, not in `blog/`.

### Search

```bash
mdvs search "how to get in touch" notes/
```

```
Searched "how to get in touch" — 3 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ how to get in touch                               │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ team/alice.md                                     │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.612                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ lines                    │ 5-8                                               │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ text                     │ Alice leads the backend team. Reach her at        │
│                          │ alice@example.com or on Slack.                    │
└──────────────────────────┴───────────────────────────────────────────────────┘

...
```

`alice.md` doesn't contain "get in touch" — mdvs finds it by meaning, not keywords. Filter with SQL on frontmatter:

```bash
mdvs search "rust" notes/ --where "draft = false"
```

No config files to write. No models to download manually. No services to start.

> **Try it on your own files:**
> ```bash
> cargo install mdvs
> mdvs init your-notes/
> mdvs search "your query" your-notes/
> ```
>
> Or explore the repo's [example_kb/](example_kb/) — 43 files across 8 directories with type widening, nullable fields, nested objects, and deliberate edge cases.

## Features

- **Schema inference** — types (boolean, integer, float, string, arrays, nested objects), path constraints (allowed/required per directory), nullable detection. All automatic.
- **Frontmatter validation** — wrong types, disallowed fields, missing required fields, null violations. Four independent checks, path-aware.
- **Semantic search** — instant vector search using lightweight [Model2Vec](https://minish.ai/) static embeddings. Default model is ~30MB. No GPU, no API keys.
- **SQL filtering** — `--where` clauses on any frontmatter field, powered by [DataFusion](https://datafusion.apache.org/). Arrays, nested objects, LIKE, IS NULL — full SQL.
- **Incremental builds** — only changed files are re-embedded. Unchanged files keep their chunks. If nothing changed, the model isn't even loaded.
- **Auto pipeline** — `search` auto-builds the index. `build` auto-updates the schema. One command does everything: `mdvs search "query"`.
- **CI-ready** — `mdvs check` returns exit code 1 on violations. Add it to your pipeline to enforce frontmatter consistency across contributors.
- **JSON output** — all commands support `--output json` for scripting and CI.

## Commands

| Command | Description |
|---------|-------------|
| `init`  | Scan files, infer schema, write `mdvs.toml` |
| `check` | Validate frontmatter against schema |
| `update` | Re-scan and update field definitions |
| `build` | Validate + embed + write search index |
| `search` | Semantic search with optional SQL filtering |
| `info`  | Show config and index status |
| `clean` | Delete search index |

## Documentation

Full documentation at [edochi.github.io/mdvs](https://edochi.github.io/mdvs/).

## License

[MIT](LICENSE)
