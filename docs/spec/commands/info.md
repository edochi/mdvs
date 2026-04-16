# `mdvs info`

Display project configuration and index status.

## Pipeline

`cmd/info.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`
2. **Auto-update** — if `[check].auto_update` is true, runs `update::run()` first
3. **Scan** — `ScannedFiles::scan()` for field prevalence counts
4. **Read index** — if `.mdvs/` exists, read `BuildMetadata` and `IndexStats` (file count, chunk count)
5. **Build field list** — for each `TomlField`: name, type, allowed, required, nullable, file count, hints

Returns `InfoOutcome` with `scan_glob`, `files_on_disk`, `fields: Vec<InfoField>`, `ignored_fields`, `index: Option<IndexInfo>`.

## Key points

- **Read-only** — never modifies config or index.
- **Field hints** — `FieldHint` enum detects special characters in field names (single quotes, double quotes, spaces) and suggests escaping for `--where` queries.
- **Index section** — shows model name, revision, chunk size, file/chunk counts, and build timestamp. Absent if `.mdvs/` doesn't exist.
