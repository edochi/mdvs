# `mdvs update`

Re-scan files, infer field changes, and update `mdvs.toml`. Pure inference — no build step.

## Pipeline

`cmd/update.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`
2. **Pre-check** — validate `--with` requires named fields; validate `--with` value list (no `none` mixed with other kinds; pairwise compatibility via `with_kinds_conflict()`)
3. **Scan** — `ScannedFiles::scan(path, &config.scan)`
4. **Infer** — `InferredSchema::infer(&scanned)` — full inference (types, paths, distinct values)
5. **Partition** — split config fields into `protected` (keep) and `targets` (reinfer):
   - No reinfer → all protected, empty targets (only new fields discovered)
   - `reinfer field1 field2` → named fields are targets, rest protected
   - `reinfer` (no fields) → all are targets
6. **Compare** — for each inferred field: if protected → skip; if in ignore → skip; else construct `TomlField` with constraints, compare against old definition → added/changed/unchanged/removed
7. **Write** — update `config.fields.field` with new list, write TOML (unless dry_run or no changes)

Returns `UpdateOutcome` with `files_scanned`, `added`, `changed`, `removed`, `unchanged`, `dry_run`.

## Constraint inference in reinfer

The `ReinferArgs.with: Vec<WithKind>` field drives constraint construction. `WithKind` is a CLI-local enum with variants `Categorical`, `Range`, `None`.

When constructing `TomlField` for reinferred fields:

- **No reinfer** (default mode) → `constraints: None` (no constraint changes on new fields)
- **`with` contains `None`** → `constraints: None` (strip all)
- **`with` is empty** → `infer_constraints(&inf, max_cat, min_rep)` (heuristic default — currently categorical only)
- **`with` is non-empty (no `None`)** → for each kind, force-infer:
  - `Categorical` → `force_categorical(&inf)` (all distinct values as categories, skip heuristic threshold)
  - `Range` → `infer_range(&inf)` (min/max from observed numeric values)

`force_categorical()` and `infer_range()` are in `cmd/update.rs` and `discover/infer/constraints/range.rs` respectively. Both check type applicability before producing values.

`with_kinds_conflict()` in `cmd/update.rs` defines pairwise CLI-level incompatibility (currently only `Categorical` ↔ `Range`). This is independent of the deeper `ConstraintKind::conflicts_with()` validation that runs at config load time, but the rules align.
