# `mdvs update`

Re-scan files, infer field changes, and update `mdvs.toml`. Pure inference — no build step.

## Pipeline

`cmd/update.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`
2. **Pre-check** — validate reinfer field names exist in config; validate `--categorical`/`--no-categorical` require named fields
3. **Scan** — `ScannedFiles::scan(path, &config.scan)`
4. **Infer** — `InferredSchema::infer(&scanned)` — full inference (types, paths, distinct values)
5. **Partition** — split config fields into `protected` (keep) and `targets` (reinfer):
   - No reinfer → all protected, empty targets (only new fields discovered)
   - `reinfer field1 field2` → named fields are targets, rest protected
   - `reinfer` (no fields) → all are targets
6. **Compare** — for each inferred field: if protected → skip; if in ignore → skip; else construct `TomlField` with constraints, compare against old definition → added/changed/unchanged/removed
7. **Write** — update `config.fields.field` with new list, write TOML (unless dry_run or no changes)

Returns `UpdateOutcome` with `files_scanned`, `added`, `changed`, `removed`, `unchanged`, `dry_run`.

## Categorical inference in reinfer

When constructing `TomlField` for reinferred fields, constraints are determined by the `ReinferArgs`:

- **No reinfer** (default mode) → `constraints: None` (no constraint changes on new fields)
- **`--no-categorical`** → `None` (strip categories)
- **`--categorical`** → `force_categorical(&inf)` (all distinct values as categories, skip heuristic)
- **Default heuristic** → `infer_constraints(&inf, max_cat, min_rep)` where thresholds come from `--max-categories`/`--min-repetition` flags or `config.fields` defaults

`force_categorical()` at `cmd/update.rs` checks type applicability (String/Integer/Array) and collects all distinct values as sorted `toml::Value` categories. Unlike the heuristic, it has no cardinality or repetition threshold.
