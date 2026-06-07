# mdvs — Markdown Validation & Search

<div align="center">

**An in-process validation & search engine for markdown documents — schema inference, frontmatter validation, and local semantic search.**

[![CI status](https://github.com/edochi/mdvs/actions/workflows/ci.yml/badge.svg)](https://github.com/edochi/mdvs/actions/workflows/ci.yml)
[![Crates.io version](https://img.shields.io/crates/v/mdvs.svg?color=orange)](https://crates.io/crates/mdvs)
[![Documentation](https://img.shields.io/badge/docs-mdBook-green.svg)](https://edochi.github.io/mdvs/)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

[Why mdvs?](#why-mdvs) • [What it does](#what-it-does) • [Installation](#install) • [Documentation](https://edochi.github.io/mdvs/) • [Example Vault](example_kb/)

</div>

<p align="center">
  <img src="assets/demo.gif" alt="mdvs: init, check, validate, build, search" width="800">
</p>

## Why mdvs?

mdvs is useful when you have a markdown corpus with structured frontmatter. Some common cases:

- **Obsidian vaults** — typed-frontmatter notes you want to keep validated and searchable, all local.
- **Knowledge bases maintained with LLM agents** — mdvs is the typed database the agent reads from (hybrid search with SQL `--where` filters) and validates against (`mdvs check` after each turn). Everything via `--output json`.
- **Docs-as-code repos** (Hugo, MkDocs, Astro) — frontmatter consistency enforced in CI; JSON Schema export for downstream tools.

## What it does

- **Infers a typed schema from your existing files.** No config to write — point it at a directory and it figures out which fields belong where.
- **Validates frontmatter.** Catches wrong types, missing required fields, and disallowed locations. Path-scoped rules: `role` can be required in `team/` but not in `blog/`.
- **Multi-format frontmatter.** YAML (`---`), TOML (`+++`), or JSON (`{...}`), auto-detected per file. Mix freely within one vault.
- **Hybrid search.** Vector similarity + BM25 full-text + RRF fusion. SQL `--where` filters on typed frontmatter: `--where "status = 'published' AND date > '2026-05-01'"`.
- **JSON Schema interop.** `mdvs export-jsonschema` emits a JSON Schema 2020-12 document; `mdvs init --from-jsonschema` imports one.
- **Runs entirely in-process.** Local files, single binary. No API keys, no vector-DB cluster, no GPU.
- **Incremental builds.** Only changed files are re-embedded. If nothing changed, the model isn't even loaded.
- **Auto-pipeline.** `search` auto-builds the index if needed. `build` auto-updates the schema before embedding.
- **Agent-callable and CI-ready.** `--output json` on every command. Deterministic exit codes (`0` = success, `1` = violations, `2` = error).

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

`title`, `tags`, and `draft` are frontmatter fields. mdvs treats them as a typed database — and your directory structure is part of the schema.

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

## Built with

- **Rust** — mdvs is written in Rust; the CLI is a single static binary.
- **[LanceDB](https://lancedb.com/)** — backs storage and search. Cosine vector search, BM25 full-text, and RRF hybrid all run natively against the Lance dataset.
- **[Model2Vec](https://minish.ai/)** — static embedding models; the default is `potion-base-8M` (~60 MB, CPU-only, no GPU).
- **[`jsonschema`](https://crates.io/crates/jsonschema)** — JSON Schema 2020-12 validator. mdvs translates your `mdvs.toml` into a canonical JSON Schema document and validates frontmatter values through per-field validators compiled from it.
- **[`pulldown-cmark`](https://crates.io/crates/pulldown-cmark)** — markdown parsing; used to extract plain text from each chunk before embedding.
- **[`text-splitter`](https://crates.io/crates/text-splitter)** — semantic-aware chunker that splits the markdown body along heading and paragraph boundaries.
- **[`gray_matter`](https://crates.io/crates/gray_matter)** — YAML and TOML frontmatter extraction. JSON frontmatter is parsed natively via `serde_json` to handle Hugo's bare-braces convention.

## Documentation

Full documentation at [edochi.github.io/mdvs](https://edochi.github.io/mdvs/).

## License

[MIT](LICENSE)
