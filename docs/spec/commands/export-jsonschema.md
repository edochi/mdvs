# `mdvs export-jsonschema`

Translate the `[fields]` block of `mdvs.toml` into a JSON Schema 2020-12 document. Round-trips losslessly with `mdvs init --from-jsonschema`.

## Pipeline

`cmd/export_jsonschema.rs` → `run(path, format, output_file)`

1. **Read config** — `MdvsToml::read(path)` + `validate()`
2. **Translate** — `dsl_to_canonical(&config)` (`schema/json_schema.rs`) emits a JSON Schema 2020-12 document
3. **Serialize** — `--format json` (default) uses `serde_json::to_string_pretty`; `--format toml` uses `tomljson::to_string`
4. **Write** — to `--output-file FILE` if provided, else stdout

Returns `ExportJsonschemaOutcome` with destination + format. When writing to stdout, the summary block is suppressed so the output is directly pipeable.

## Flags

| Flag | Default | Behavior |
|------|---------|----------|
| `[PATH]` | `.` | Project directory containing `mdvs.toml` |
| `--format json\|toml` | `json` | Output format. `toml` uses the workspace `tomljson` crate |
| `--output-file FILE` | (stdout) | Write to a file instead of stdout |

## Extension keys

mdvs-specific metadata that JSON Schema 2020-12 doesn't model is carried in `x-mdvs` extension objects:

- **Schema level** — `x-mdvs.preprocess` (top-level preprocessor stages, reserved), `x-mdvs.definitions`
- **Property level** — `x-mdvs.allowed` (path-scoping globs), `x-mdvs.required` (path-scoping globs), `x-mdvs.preprocess` (Stage 2 preprocessor list for this field)

These keys are ignored by generic JSON Schema validators and round-tripped by `canonical_to_dsl`.

## Round-trip guarantee

`mdvs export-jsonschema ./project --output-file out.json` followed by `mdvs init --from-jsonschema out.json ./reborn` reproduces the original `[[fields.field]]` definitions including:

- Field types (strict — `String` ≠ permissive set)
- Constraints (`categories`, `min`, `max`, `min_length`, `max_length`, `pattern`)
- Path-scoping (`allowed`, `required`)
- Preprocessor arrays (`preprocess`)
- The `[fields].ignore` list (carried in `x-mdvs.definitions`)

Build sections (`[embedding_model]`, `[chunking]`, `[search]`) and scan config are **not** exported — JSON Schema is only the fields contract.

## Not exported

- `[scan]`, `[embedding_model]`, `[chunking]`, `[search]`, `[update]`, `[check]`, `[build]` — these live in `mdvs.toml` only.
- Inference thresholds in `[fields]` (`max_distinct_for_categorical`, etc.) — these are inference hyperparameters, not part of the schema.

See [architecture.md](../architecture.md#validation-pipeline) for the translator and gate.
