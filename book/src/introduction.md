# Introduction

mdvs treats your markdown directory like a database. It scans your files, infers a typed schema from frontmatter, validates it, and builds a local search index — all in a single binary with no external services.

Not a document database. A database *for* documents.

## The problem

Markdown directories grow organically. You start with a few notes, add frontmatter when it's useful, and eventually have hundreds of files with inconsistent metadata. Tags are misspelled. Required fields are missing. You can't find anything without `grep`.

mdvs gives you structure without forcing you to change how you write.

## Two layers

mdvs has two distinct capabilities that work independently:

**Validation** — Scan your files, infer what frontmatter fields exist, where they appear, and what types they have. Write the result to `mdvs.toml`. Then validate files against that schema. No model, no index, nothing to download.

**Search** — Chunk your markdown, embed it with a lightweight local model (8MB, no GPU), store the vectors in Parquet files, and query with natural language. Filter results on any frontmatter field using SQL.

You can use validation without search. Many workflows only need `init`, `check`, and `update`.

## The database analogy

If your markdown directory is a database:

| Concept | Database | mdvs |
|---|---|---|
| Define structure | `CREATE TABLE` | `mdvs init` |
| Enforce constraints | Constraint validation | `mdvs check` |
| Evolve structure | `ALTER TABLE` | `mdvs update` |
| Create an index | `CREATE INDEX` | `mdvs build` |
| Query | `SELECT ... WHERE ... ORDER BY` | `mdvs search --where` |

Two artifacts: `mdvs.toml` (your schema, committed) and `.mdvs/` (the search index, gitignored).

## What this book covers

This book uses a fictional research lab knowledge base ([example_kb](https://github.com/edochi/mdvs/tree/main/example_kb)) as a running example. Every command, every output, every query is real and reproducible.

- **[Getting Started](./getting-started.md)** — Install mdvs and run it on the example vault
- **[Concepts](./concepts.md)** — How schema inference, types, and validation work
- **[Commands](./commands/init.md)** — Full reference for all 7 commands
- **[Configuration](./configuration.md)** — The `mdvs.toml` file explained
- **[Search Guide](./search-guide.md)** — SQL filtering, array queries, and ranking
- **[Recipes](./recipes/obsidian.md)** — Obsidian setup, CI integration
