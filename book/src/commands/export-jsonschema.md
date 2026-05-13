# export-jsonschema

Translate `mdvs.toml`'s `[fields]` block into a JSON Schema 2020-12 document.

## Usage

```bash
mdvs export-jsonschema [path] [flags]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--format json\|toml` | `json` | Output format. `toml` produces a TOML serialization of the same JSON Schema |
| `--output-file FILE` | (stdout) | Write to a file instead of stdout |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`export-jsonschema` reads `mdvs.toml`, takes the `[fields]` block, and translates it into a JSON Schema 2020-12 document. Field types, constraints, path-scoping, and preprocessor stages are all preserved. The output is a valid JSON Schema that any standards-compliant validator can consume.

Build configuration (`[embedding_model]`, `[chunking]`, `[search]`) and scan settings are **not** included — JSON Schema only describes the field contract.

### Round-tripping with init

`export-jsonschema` and `init --from-jsonschema` are designed to round-trip losslessly:

```bash
mdvs export-jsonschema ./project --output-file fields.json
mdvs init ./reborn --from-jsonschema fields.json
```

The new `mdvs.toml` reproduces the original `[[fields.field]]` definitions:

- Field types (strict — `String` is `String`, not a permissive set)
- Constraints (`categories`, `min`/`max`, `min_length`/`max_length`, `pattern`)
- Path-scoping (`allowed`, `required`)
- Preprocessor arrays (`preprocess = ["coerce-to-string"]`, etc.)
- The `[fields].ignore` list

mdvs-specific metadata that JSON Schema 2020-12 doesn't model is carried in `x-mdvs.*` extension keys; generic JSON Schema validators ignore them, and `init --from-jsonschema` reads them back.

## Examples

### Export to stdout (pipeable)

```bash
mdvs export-jsonschema example_kb | jq '.properties | keys'
```

When writing to stdout, the summary banner is suppressed so the output is directly pipeable.

### Export to a file

```bash
mdvs export-jsonschema example_kb --output-file fields.json
```

```
Exported schema for 37 field(s) → fields.json (json)
```

### Export as TOML

```bash
mdvs export-jsonschema example_kb --format toml --output-file fields.toml
```

The TOML output is the same JSON Schema, serialized via the workspace `tomljson` crate. It's interchangeable with the JSON form — `init --from-jsonschema fields.toml` produces the same result as the JSON file.

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `mdvs.toml is invalid` | TOML parsing or schema error |
| `failed to write` | Output file path is not writable |
