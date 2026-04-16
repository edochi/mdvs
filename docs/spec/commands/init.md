# `mdvs init`

Scan a directory, infer a typed schema, and write `mdvs.toml`.

## Pipeline

`cmd/init.rs` → `run()`

1. **Pre-checks** — directory exists, config path resolved, `--force` deletes existing config + `.mdvs/`
2. **Scan** — `ScannedFiles::scan(path, &scan_config)` (`discover/scan.rs`)
3. **Infer** — `InferredSchema::infer(&scanned)` (`discover/infer/mod.rs`) — type widening, path inference, distinct value collection
4. **Build config** — `MdvsToml::from_inferred(&schema, scan_config)` (`schema/config.rs:171`) — converts `InferredField` to `TomlField`, runs `infer_constraints()` with default thresholds to populate categorical constraints
5. **Write** — `config.write(&path)` — serializes TOML, post-processes complex types to inline tables

Returns `InitOutcome` with `files_scanned`, `fields: Vec<DiscoveredField>`, `dry_run`.

## Key points

- **Schema-only** — init never downloads a model, never creates `.mdvs/`, never embeds.
- **Categorical inference** — `infer_constraints()` runs with defaults (max_categories=10, min_repetition=3). Qualifying fields get `[fields.field.constraints].categories`.
- **`init --force` vs `update reinfer`** — init rewrites the entire config (all sections). `update reinfer` re-infers only `[fields]`, preserving all other config.

See [inference.md](../inference.md) for the inference algorithm.
