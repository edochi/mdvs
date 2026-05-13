# Commands

mdvs provides eight commands covering the full workflow — from schema setup to search.

**Schema & validation:**
- **[init](./commands/init.md)** — Scan a directory, infer a typed schema, and write `mdvs.toml` (or import via `--from-jsonschema`)
- **[check](./commands/check.md)** — Validate frontmatter against the schema (optionally `--jsonschema` to override)
- **[update](./commands/update.md)** — Re-scan files, infer new fields, and update the schema
- **[export-jsonschema](./commands/export-jsonschema.md)** — Translate `mdvs.toml`'s `[fields]` into a JSON Schema 2020-12 document

**Search index:**
- **[build](./commands/build.md)** — Validate, embed, and write the search index
- **[search](./commands/search.md)** — Query the index with natural language

**Utilities:**
- **[info](./commands/info.md)** — Show config and index status
- **[clean](./commands/clean.md)** — Delete the search index
