# `mdvs init`

Scan a directory, infer a typed schema, and write `mdvs.toml`. Optionally import the schema from an external JSON Schema file via `--from-jsonschema`.

## Pipeline (default: infer from scan)

`cmd/init.rs` → `run()`

1. **Pre-checks** — directory exists, config path resolved, `--force` deletes existing config + `.mdvs/`
2. **Scan** — `ScannedFiles::scan(path, &scan_config)` (`discover/scan.rs`)
3. **Infer** — `InferredSchema::infer(&scanned)` (`discover/infer/mod.rs`) — type widening, path inference, distinct value collection, observed-types tracking. Fields with unrepresentable shapes (`Array(Object{...})`) are partitioned into `schema.dropped`.
4. **Warn** — `schema.emit_dropped_warnings()` prints one stderr line per dropped field with the field name, reason, and first-observed file path.
5. **Build config** — `MdvsToml::from_inferred(&schema, scan_config)` (`schema/config.rs`) — converts representable `InferredField`s to `TomlField`s, runs `infer_constraints()` for categorical fields, runs `infer_value_stages()` to populate `preprocess` arrays from observed widening events
6. **Write** — `config.write(&path)` — serializes TOML; `type` is written as a function-style string (`"Array(String)"`).

Returns `InitOutcome` with `files_scanned`, `fields: Vec<DiscoveredField>`, `dry_run`.

## Pipeline (--from-jsonschema PATH)

Skips scan + infer; the external file is the source of truth for fields. `init_from_schema()` in `cmd/init.rs`:

1. **Load** — `load_schema(path)` from `schema/load.rs` parses by extension (`.json` via `serde_json`, `.toml` via `tomljson`)
2. **Gate** — `validate_mdvs_schema(&schema)` (`schema/json_schema.rs`) checks the allow-list of supported keywords and rejects unsupported features (`oneOf`, `$ref`, `format`, etc.) with explanatory messages
3. **Translate** — `canonical_to_dsl(&schema)` translates JSON Schema 2020-12 back into `Vec<TomlField>` + ignore list, reading `x-mdvs.allowed`, `x-mdvs.required`, `x-mdvs.preprocess`
4. **Assemble** — `MdvsToml::default_with_fields(fields, ignore)` synthesizes a minimal config (no `[embedding_model]`/`[chunking]`/`[search]` build sections unless `--auto-build`)
5. **Write** — same as default flow

## Key points

- **Schema-only** — init never downloads a model, never creates `.mdvs/`, never embeds.
- **Categorical inference** — `infer_constraints()` runs with defaults (max_categories=10, min_repetition=3). Qualifying fields get `[fields.field.constraints].categories`.
- **Preprocessor inference** — observed type-widening events drive `[fields.field].preprocess`. No implicit defaults: `preprocess = []` means strict.
- **`init --force` vs `update reinfer`** — init rewrites the entire config (all sections). `update reinfer` re-infers only `[fields]`, preserving all other config.
- **Round-trip with `mdvs export-jsonschema`** — exporting then re-importing reproduces the original `[[fields.field]]` definitions including constraints, path-scoping, and `preprocess` arrays (preserved via `x-mdvs.*` extension keys).

See [inference.md](../inference.md) for the inference algorithm and [architecture.md](../architecture.md#validation-pipeline) for the translation gates.
