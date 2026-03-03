# Introduction

mdvs (**Markdown Validation & Search**) treats your markdown directory like a database.

It scans your files, infers a typed schema from frontmatter, validates it, and builds a local search index — all in a single binary with zero external dependencies.

## What you get

- **A schema for your markdown files.** mdvs discovers what fields your frontmatter has, what types they are, and where they appear. It writes this to `mdvs.toml` — a schema you can version-control and enforce.

- **Validation that catches mistakes.** Missing required fields, wrong types, fields that appear where they shouldn't — mdvs checks them all.

- **Semantic search with SQL filtering.** Find files by meaning, not just keywords. Filter results on any frontmatter field using familiar SQL syntax.

## The database analogy

If your markdown directory is a database, then:

| mdvs command | Database equivalent |
|---|---|
| `init` | `CREATE TABLE` — infer a schema from your files |
| `check` | Constraint validation — catch violations |
| `update` | `ALTER TABLE` — detect new fields as files evolve |
| `build` | Create an index — chunk, embed, and store |
| `search` | `SELECT ... WHERE ... ORDER BY similarity` |

Two artifacts: `mdvs.toml` (your schema, committed) and `.mdvs/` (the search index, gitignored).

## Next steps

Ready to try it? Head to [Getting Started](./getting-started.md).
