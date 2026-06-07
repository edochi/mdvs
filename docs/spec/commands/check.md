# `mdvs check`

Validate frontmatter against the schema. Read-only — never modifies files or config.

## Pipeline

`cmd/check/mod.rs` → `run()` → `pub fn validate()` (in `validate.rs`).

1. **Resolve config** — `resolve_check_config()` reads `mdvs.toml` and, if `--jsonschema PATH` is provided, overrides the `[fields]` section. When no `mdvs.toml` exists but `--jsonschema` is given, synthesizes a default via `MdvsToml::default_with_fields(fields, ignore)` so downstream code sees a normal `MdvsToml`.
2. **Auto-update** — if `[check].auto_update` is true (default) and no `--jsonschema` override, runs inference + merges any newly-discovered fields into the config. Fields with unrepresentable shapes (`Array(Object{...})`) are partitioned out by `InferredSchema::infer` and surfaced via `emit_dropped_warnings()` to stderr; they are NOT added to the config.
3. **Scan** — `ScannedFiles::scan(path, &config.scan)` — `ScannedFile.frontmatter_error` carries any YAML→JSON representation failure.
4. **Build validators + field metas + pipeline** — once per `validate()` call:
   - `FieldValidators::build(config)` compiles one `jsonschema::Validator` per leaf via `dsl_to_canonical` + `extract_leaf_schemas` (keyed by dotted name).
   - `build_field_metas(config)` precomputes per-field `FieldMeta` (compiled `GlobSet`s for `allowed` / `required`, plus a cached `FieldType::try_from(field.field_type)`). Without this, the inner `(field, file)` loop would call `Glob::new` and `FieldType::try_from` tens of thousands of times.
   - `Pipeline::for_config(config)` builds the Stage 2 preprocessor pipeline per field.
   - Per-file path strings are precomputed once so `path.display().to_string()` doesn't run inside the per-required-field inner loop.
5. **Validate** — for each file:
   - **Frontmatter errors** — if `frontmatter_error` is set, emit `ViolationKind::FrontmatterUnrepresentable` with sentinel field `<frontmatter>`.
   - **Per-field values** — `check_field_values` runs the Stage 2 pipeline to normalize, then takes a `validator.is_valid()` fast path: when no errors exist (the common case), only the path-scoping `Disallowed` check fires. Only fields whose validator returned errors go through the full `validator.iter_errors()` + `map_validation_error` path. The strict-Float precheck (`preprocess::strict_subtype_check`) runs before either of the above. Path-scoping uses the precomputed `FieldMeta::allowed` `GlobSet`.
   - **Required fields** — `check_required_fields` per-field iteration over the precomputed `FieldMeta::required` `GlobSet`.
6. **Collect** — `collect_violations()` groups by `ViolationKey { field, kind, rule }`. Output sort is byte-stable: outer sort `(field, kind, rule)` where `kind` uses `ViolationKind`'s `Ord` derive (declaration order: `MissingRequired` < `WrongType` < `Disallowed` < `NullNotAllowed` < `InvalidCategory` < `OutOfRange` < `FrontmatterUnrepresentable`); inner sort files within each violation by path. The byte-stability contract matters for CI consumers that diff `mdvs check` output across runs.

Returns `CheckOutcome` with `files_checked`, `violations: Vec<FieldViolation>`, `new_fields: Vec<NewField>`.

## Validation engine (post-Wave-B)

Validation runs through the `jsonschema` crate (v0.46). Hand-rolled per-value validators have been removed.

- **Translation** — `dsl_to_canonical(config)` translates `[fields]` into a JSON Schema 2020-12 document. Per-field validators are compiled once per `validate()` call, keyed by the field's full dotted name (e.g. `calibration.baseline.wavelength`). Extracted from the canonical schema's nested `properties` tree via `extract_leaf_schemas` (TODO-0097 step 4).
- **Dotted-path navigation** — `navigate_dotted(frontmatter, "cal.baseline.wave")` walks the YAML's nested Object structure to retrieve the leaf value. An absent intermediate counts as the leaf being absent (handled by `check_required_fields`).
- **Strict subtype precheck** — `preprocess::strict_subtype_check` runs in Rust before the preprocessor pipeline. Currently enforces strict-Float (rejects integer-backed values on Float / Array(Float) fields unless `widen-int-to-float` is in `preprocess`). See [architecture.md](../architecture.md#strict-subtype-prechecks) for the rationale.
- **Preprocessing** — each field's `preprocess` array (e.g. `["coerce-to-string"]`) runs before jsonschema, transforming the value via `Pipeline::apply_to_value`.
- **Format validation** — Validators are built with `jsonschema::options().should_validate_formats(true)`. Two formats are validated at runtime: `date` (RFC 3339 full-date) and `date-time` (RFC 3339 datetime). Other format values are rejected by the schema gate, so they can't reach the validator. Format failures map to `WrongType` with rule `format <name>` (TODO-0007).
- **Error mapping** — `map_validation_error` is an exhaustive match over `ValidationErrorKind`; new variants in future jsonschema versions cause a compile error rather than a silent fallback.
- **Array-of-mappings against a scalar `Array` field** — fires the existing `WrongType` violation (the element is a JSON Object, not the expected scalar type). No special `Array(Object)` handling is needed in validation because the on-disk type vocabulary doesn't include it (TODO-0155).

See [architecture.md](../architecture.md#validation-pipeline) for the full pipeline and error mapping table.

## `--jsonschema` override

`mdvs check --jsonschema PATH` replaces the `[fields]` block for this run. Useful for one-off validation against a contract without editing `mdvs.toml`. The file is loaded via `schema/load.rs` (extension-dispatched: `.json` / `.toml`) and gated via `validate_mdvs_schema`.

## Violation grouping

`ViolationKey { field, kind, rule }` groups files violating the same rule. Multiple files with the same violation → one `FieldViolation` entry with `files: Vec<ViolatingFile>`. Detail (e.g., `got String`) lives on `ViolatingFile`, not the key.

## ViolationKind values

`MissingRequired`, `WrongType`, `Disallowed`, `NullNotAllowed`, `InvalidCategory`, `OutOfRange`, `FrontmatterUnrepresentable`.
