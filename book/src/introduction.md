# Introduction

mdvs treats your markdown directory like a database. It scans your files, infers a typed schema from frontmatter, validates it, and builds a local search index — all in a single binary with no external services.

Not a document database. A database *for* documents.

## The problem

Markdown directories grow organically. You start with a few notes, add frontmatter when it's useful, and eventually have hundreds of files with inconsistent metadata. Tags are misspelled. Required fields are missing. You can't find anything without `grep`.

mdvs gives you structure without forcing you to change how you write.

## Frontmatter

Frontmatter is the YAML block between `---` fences at the top of a markdown file. It stores structured metadata alongside your content:

```yaml
---
title: "Experiment A-017: SPR-A1 baseline calibration"    # String
status: completed                                         # String
author: Giulia Ferretti                                   # String
draft: false                                              # Boolean
priority: 2                                               # Integer
drift_rate: 0.023                                         # Float
tags:                                                     # String[]
  - calibration
  - SPR-A1
  - baseline
---
# Your markdown content starts here...
```

mdvs recognizes these types automatically. When it scans your files, it infers the type of each field from the values it finds — no configuration needed.

## Two layers

mdvs has two distinct capabilities that work independently:

**Validation** — Scan your files, infer what frontmatter fields exist, where they appear, and what types they have. Write the result to `mdvs.toml`. Then validate files against that schema. No model, no index, nothing to download.

**Search** — Chunk your markdown, embed it with a lightweight local model, store the vectors in Parquet files in `.mdvs/`, and query with natural language. Filter results on any frontmatter field using standard SQL.

You need validation without search? Run `mdvs init --suppress-auto-build`, customise the fields in `mdvs.toml`, and run `mdvs check` to validate your files. 

You want search without validation? Just run `mdvs init` and `mdvs search` to get going. The inferred schema is used to extract metadata for search results, but you don't have to worry about it if you don't want to.

Use them together for the best experience, or separately if that's what you need.

## Using a nested directory of markdown files as a database

You can think of mdvs as a layer on top of your markdown files that gives you database-like capabilities. Here's a rough mapping of concepts and commands:

| Concept | Database | mdvs |
|---|---|---|
| Define structure | `CREATE TABLE` | `mdvs init` |
| Enforce constraints | Constraint validation | `mdvs check` |
| Evolve structure | `ALTER TABLE` | `mdvs update` |
| Create an index | `CREATE INDEX` | `mdvs build` |
| Query | `SELECT ... WHERE ... ORDER BY` | `mdvs search --where` |

Two artifacts: `mdvs.toml` (your schema, to be committed) and `.mdvs/` (the search index, can be ignored by version control).

## What this book covers

This book uses a fictional research lab knowledge base ([example_kb](https://github.com/edochi/mdvs/tree/main/example_kb)) as a running example. Every command, every output, every query is real and reproducible.

- **[Getting Started](./getting-started.md)** — Install mdvs and run it on the example vault
- **[Concepts](./concepts.md)** — How schema inference, types, and validation work
- **[Commands](./commands/init.md)** — Full reference for all 7 commands
- **[Configuration](./configuration.md)** — The `mdvs.toml` file explained
- **[Search Guide](./search-guide.md)** — SQL filtering, array queries, and ranking
- **[Recipes](./recipes/obsidian.md)** — Obsidian setup, CI integration
