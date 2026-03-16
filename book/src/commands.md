# Commands

mdvs provides seven commands covering the full workflow — from schema setup to search.

**Schema & validation:**
- **[init](./commands/init.md)** — Scan a directory, infer a typed schema, and write `mdvs.toml`
- **[check](./commands/check.md)** — Validate frontmatter against the schema
- **[update](./commands/update.md)** — Re-scan files, infer new fields, and update the schema

**Search index:**
- **[build](./commands/build.md)** — Validate, embed, and write the search index
- **[search](./commands/search.md)** — Query the index with natural language

**Utilities:**
- **[info](./commands/info.md)** — Show config and index status
- **[clean](./commands/clean.md)** — Delete the search index
