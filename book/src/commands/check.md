# check

Validate frontmatter against the schema.

## Usage

```bash
mdvs check [path]
```

## Flags

| Flag | Default | Description |
|---|---|---|
| `path` | `.` | Directory containing `mdvs.toml` |
| `--no-update` | | Skip auto-update before validating |

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`check` reads `mdvs.toml`, scans every markdown file, and validates each field value against the declared constraints.

By default, `check` auto-updates the schema before validating (see [`[check].auto_update`](../configuration.md#check)). Use `--no-update` to skip this and validate against the current `mdvs.toml` as-is.

It reports four kinds of violations:

- **`WrongType`** — value doesn't match the declared `type`
- **`Disallowed`** — field appears in a file whose path doesn't match any `allowed` glob
- **`MissingRequired`** — file matches a `required` glob but the field is absent
- **`NullNotAllowed`** — field is `null` but `nullable = false`

Fields not in `mdvs.toml` (and not in the `ignore` list) are reported as **new fields** — these are informational and don't count as violations.

`check` is read-only — it never modifies `mdvs.toml` or any files. See [Validation](../concepts/validation.md) for the full rules, including type leniency and null handling.

## Output

### Compact (default)

When everything passes:

```bash
mdvs check example_kb
```

```
Checked 43 files — no violations
```

When violations are found, each violation is shown as a key-value table with the field name, violation kind, the violated rule, and the affected files:

```
Checked 43 files — 3 violation(s)

Violations (3):
┌ drift_rate ──────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Null value not allowed                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ not nullable                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/alpha/notes/experiment-2.md              │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ priority ────────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Wrong type                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ type Integer                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/beta/notes/initial-findings.md (got Stri │
│                          │ ng)                                               │
│                          │ projects/beta/overview.md (got String)            │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ title ───────────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Missing required                                  │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ required in ["**"]                                │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ README.md                                         │
│                          │ lab-values.md                                     │
│                          │ reference/glossary.md                             │
│                          │ reference/quick-start.md                          │
│                          │ reference/tools.md                                │
│                          │ scratch.md                                        │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

`WrongType` violations include the actual type in parentheses (e.g., `got String`).

### Verbose (`-v`)

Verbose output adds pipeline timing lines before the result:

```
Read config: example_kb/mdvs.toml (3ms)
Scan: 43 files (2ms)
Validate: 43 files — 3 violation(s) (78ms)
Checked 43 files — 3 violation(s)

Violations (3):
┌ drift_rate ──────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Null value not allowed                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ not nullable                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/alpha/notes/experiment-2.md              │
└──────────────────────────┴───────────────────────────────────────────────────┘

...
```

The violation tables are identical in both modes — verbose only adds the step lines showing processing times.

## Exit codes

| Code | Meaning |
|---|---|
| `0` | All files valid — no violations |
| `1` | Violations found |
| `2` | Pipeline error (missing `mdvs.toml`, invalid config, scan failure) |

New fields don't affect the exit code — they're informational only.

## Errors

| Error | Cause |
|---|---|
| `no mdvs.toml found` | Config doesn't exist — run `mdvs init` first |
| `mdvs.toml is invalid` | TOML parsing or schema error — fix the file or run `mdvs init --force` |
