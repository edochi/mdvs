# `mdvs check`

Validate frontmatter against the schema. Read-only — never modifies files or config.

## Pipeline

`cmd/check.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()` (`schema/config.rs`)
2. **Auto-update** — if `[check].auto_update` is true (default), runs `update::run()` first to detect new fields
3. **Scan** — `ScannedFiles::scan(path, &config.scan)`
4. **Validate** — `check_field_values()` + `check_required_fields()` → accumulate into `HashMap<ViolationKey, Vec<ViolatingFile>>`
5. **Collect** — `collect_violations()` groups by field/kind/rule, sorts alphabetically

Returns `CheckOutcome` with `files_checked`, `violations: Vec<FieldViolation>`, `new_fields: Vec<NewField>`.

## Validation dispatch (`check_field_values`)

For each field in each file's frontmatter, in order:

1. **Skip** if field is in `ignore` set
2. **Known field** (in `field_map`):
   - `Disallowed` — path doesn't match any `allowed` glob
   - `NullNotAllowed` — value is null and `nullable = false`
   - `WrongType` — value doesn't match declared type (only if not null)
   - `InvalidCategory` — value not in `categories` (only if type matches and not null)
3. **Unknown field** — recorded in `new_field_paths` (informational, not a violation)

Key: constraint validation runs after type check passes. If type fails, `InvalidCategory` is skipped — no double violations.

## Violation grouping

`ViolationKey { field, kind, rule }` groups files violating the same rule. Multiple files with the same violation → one `FieldViolation` entry with `files: Vec<ViolatingFile>`. Detail (e.g., `got String`) lives on `ViolatingFile`, not the key.
