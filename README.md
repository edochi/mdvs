# mdvs — Markdown Validation & Search

<div align="center">

**An in-process validation & search engine for markdown documents — schema inference, frontmatter validation, and local semantic search.**

[![CI status](https://github.com/edochi/mdvs/actions/workflows/ci.yml/badge.svg)](https://github.com/edochi/mdvs/actions/workflows/ci.yml)
[![Crates.io version](https://img.shields.io/crates/v/mdvs.svg?color=orange)](https://crates.io/crates/mdvs)
[![Documentation](https://img.shields.io/badge/docs-mdBook-green.svg)](https://edochi.github.io/mdvs/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

[Key Features](#features) • [Installation](#install) • [Documentation](https://edochi.github.io/mdvs/) • [Example Vault](example_kb/)

</div>

<p align="center">
  <img src="assets/demo.gif" alt="mdvs: init, check, validate, build, search" width="800">
</p>

## Why mdvs?

Markdown files can have a block at the top called **frontmatter** — structured fields that describe the document. mdvs accepts YAML (`---`), TOML (`+++`), or JSON (`{...}`); the format is auto-detected per file, so mixed-format vaults work transparently.

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
cd notes/
mdvs init
```

mdvs scans every file, extracts frontmatter, and infers which fields belong where. `draft` is a `Boolean` allowed in `blog/`. `role` is a `String` required in `team/`. `date` is a `Date` (RFC 3339) allowed in `meetings/`. The directory structure is the schema.

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
mdvs check
```

```
Checked 7 files — 1 violation(s)

┌ role ────────────────────┬─────────────────────────────────────────┐
│ kind                     │ Missing required                        │
│ rule                     │ required in ["team/**"]                 │
│ files                    │ team/charlie.md                         │
└──────────────────────────┴─────────────────────────────────────────┘
```

`charlie.md` is missing `role` — but `new-post.md` isn't flagged. mdvs knows `role` belongs in `team/`, not in `blog/`.

This is especially useful when an LLM agent is doing the writing. Over a long session an agent will drift — a misnamed field here, an accidentally-stringified boolean there — and running `mdvs check` after each turn catches the drift before it compounds.

### Search

```bash
mdvs search "how to get in touch"
```

```
Searched "how to get in touch" — 3 hits

┌ #1 ──────────────────────┬─────────────────────────────────────────┐
│ file                     │ team/alice.md                           │
│ score                    │ 0.612                                   │
│ lines                    │ 5-8                                     │
│ text                     │ Alice leads the backend team. Reach her │
│                          │ at alice@example.com or on Slack.       │
└──────────────────────────┴─────────────────────────────────────────┘
```

`alice.md` doesn't contain "get in touch" — mdvs finds it by meaning, not keywords. Filter with SQL on frontmatter:

```bash
mdvs search "rust" --where "draft = false"
mdvs search "meeting notes" --where "date > '2026-05-01'"
```

The typed schema is what makes `--where` work. Without it, `tags = 'rust'` would be a fuzzy guess; with it, it's an equality check on a known-typed array column.

> **Try it on your own files:**
> ```bash
> cargo install mdvs
> cd your-notes/
> mdvs init
> mdvs search "your query"
> ```
>
> Or explore the repo's [example_kb/](example_kb/) — 43 files across 8 directories with type widening, nullable fields, nested objects, and deliberate edge cases.

## Calling mdvs from an agent

Every command supports `--output json` and returns deterministic exit codes (`0` = success, `1` = violations, `2` = error). No SDK or daemon to manage — a coding agent calls mdvs the way a shell script would.

```bash
# An agent checks its own writes:
mdvs check --output json | jq '.violations[] | select(.kind == "MissingRequired")'

# An agent queries by metadata + meaning together:
mdvs search "incident postmortem" \
  --where "status = 'published' AND severity = 'high'" \
  --output json | jq '.hits[].filename'

# An agent exports the schema to feed into a structured-output generator:
mdvs export-jsonschema --format json
```

## Features

- **Multi-format frontmatter** — YAML (`---`), TOML (`+++`), or JSON (`{...}`), auto-detected per file. Mix freely within one vault. Native TOML `Date` / `DateTime` literals are recognized.
- **Schema inference** — types (boolean, integer, float, string, RFC 3339 `Date` and `DateTime`, arrays), nested frontmatter structure exposed as dotted-name leaf fields (`calibration.baseline.wavelength`), path constraints (allowed/required per directory), nullable detection, value preprocessors. All automatic.
- **Frontmatter validation** — wrong types, disallowed fields, missing required fields, nullability, categories, numeric/length ranges, regex patterns, unrepresentable frontmatter. Powered by [`jsonschema`](https://crates.io/crates/jsonschema); your `mdvs.toml` round-trips losslessly to a JSON Schema 2020-12 document.
- **JSON Schema interop** — `mdvs export-jsonschema` translates your config into a JSON Schema document; `mdvs init --from-jsonschema` imports one.
- **Semantic, full-text, and hybrid search** — instant vector search using lightweight [Model2Vec](https://minish.ai/) static embeddings, full-text BM25 ranking, and hybrid RRF reranking, all backed by [LanceDB](https://lancedb.com/). Pick with `--mode`; default is hybrid. No GPU, no API keys, no vector-DB cluster — everything runs in-process.
- **SQL filtering** — `--where` clauses on any frontmatter field, backed by LanceDB's native filter. Arrays, nested objects, `LIKE`, `IS NULL` — full SQL.
- **Incremental builds** — only changed files are re-embedded. Unchanged files keep their chunks. If nothing changed, the model isn't even loaded.
- **Auto pipeline** — `search` auto-builds the index. `build` auto-updates the schema. One command does everything: `mdvs search "query"`.
- **CI-ready** — `mdvs check` returns exit code 1 on violations. Add it to your pipeline to enforce frontmatter consistency across contributors.
- **JSON output** — all commands support `--output json` for scripting and agent use.

## Commands

| Command | Description |
|---------|-------------|
| `init`  | Scan files, infer schema, write `mdvs.toml` (or import via `--from-jsonschema`) |
| `check` | Validate frontmatter against schema (optionally `--jsonschema` to override) |
| `update` | Re-scan and update field definitions |
| `build` | Validate + embed + write search index |
| `search` | Semantic search with optional SQL filtering |
| `info`  | Show config and index status |
| `clean` | Delete search index |
| `export-jsonschema` | Translate `mdvs.toml` fields into a JSON Schema 2020-12 document |
| `skill` | Print the agent skill file to stdout (for harnesses that load it as a tool description) |

## Documentation

Full documentation at [edochi.github.io/mdvs](https://edochi.github.io/mdvs/).

## License

[MIT](LICENSE)
