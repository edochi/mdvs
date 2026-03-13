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

Global flags (`-o`, `-v`, `--logs`) are described in [Configuration](../configuration.md).

## What it does

`check` reads `mdvs.toml`, scans every markdown file, and validates each field value against the declared constraints. It reports four kinds of violations:

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

When violations are found:

```
Checked 43 files — 3 violation(s), 1 new field(s)

╭──────────────────────────┬─────────────────────────────┬─────────────────────╮
│ "drift_rate"             │ NullNotAllowed              │ 1 file              │
│ "priority"               │ WrongType                   │ 2 files             │
│ "title"                  │ MissingRequired             │ 6 files             │
╰──────────────────────────┴─────────────────────────────┴─────────────────────╯

╭──────────────────────────────┬─────────────────────┬─────────────────────────╮
│ "algorithm"                  │ new                 │ 2 files                 │
╰──────────────────────────────┴─────────────────────┴─────────────────────────╯
```

Each violation row shows the field name, violation kind, and how many files are affected. New fields appear in a separate table below.

### Verbose (`-v`)

```
Checked 43 files — 3 violation(s), 1 new field(s)

╭────────────────────────────┬────────────────────────────┬────────────────────╮
│ "drift_rate"               │ NullNotAllowed             │ 1 file             │
├────────────────────────────┴────────────────────────────┴────────────────────┤
│   - "projects/alpha/notes/experiment-2.md"                                   │
╰──────────────────────────────────────────────────────────────────────────────╯
╭────────────────────────────┬─────────────────────────┬───────────────────────╮
│ "priority"                 │ WrongType               │ 2 files               │
├────────────────────────────┴─────────────────────────┴───────────────────────┤
│   - "projects/beta/notes/initial-findings.md" (got String)                   │
│   - "projects/beta/overview.md" (got String)                                 │
╰──────────────────────────────────────────────────────────────────────────────╯
╭───────────────────────┬───────────────────────────────┬──────────────────────╮
│ "title"               │ MissingRequired               │ 6 files              │
├───────────────────────┴───────────────────────────────┴──────────────────────┤
│   - "README.md"                                                              │
│   - "lab-values.md"                                                          │
│   - "reference/glossary.md"                                                  │
│   - "reference/quick-start.md"                                               │
│   - "reference/tools.md"                                                     │
│   - "scratch.md"                                                             │
╰──────────────────────────────────────────────────────────────────────────────╯

╭──────────────────────────────┬─────────────────────┬─────────────────────────╮
│ "algorithm"                  │ new                 │ 2 files                 │
├──────────────────────────────┴─────────────────────┴─────────────────────────┤
│   - "projects/beta/notes/initial-findings.md"                                │
│   - "projects/beta/notes/replication.md"                                     │
╰──────────────────────────────────────────────────────────────────────────────╯
```

Verbose output expands each violation into a record with the offending file paths. `WrongType` violations include the actual type in parentheses (e.g., `got String`).

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
