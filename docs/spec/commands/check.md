# `mdvs check`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md)

---

## Synopsis

```
mdvs check [path]
```

| Flag   | Type       | Default | Description                    |
|--------|------------|---------|--------------------------------|
| `path` | positional | `.`     | Directory containing mdvs.toml |

No other flags — check reads all config from `mdvs.toml`.

---

## Behavior

1. Read `mdvs.toml` (see [Prerequisites](#prerequisites))
2. Scan markdown files using `[scan]` config
3. For each file, for each frontmatter field:
   - If field is in `[[fields.field]]`: validate type, check `allowed` globs
   - If field is in `[fields].ignore`: skip
   - If field is not in toml at all: collect as `NewField`
4. For each `[[fields.field]]` with `required` globs: check that all matching files have the field
5. Collect `CheckResult`
6. Print result, grouped by field
7. Exit 0 if no violations, exit 1 if any violations or errors

Check is **read-only** — it never modifies the toml or any files.

New fields do not affect the exit code — they are informational only.

**Bare files:** if `include_bare_files = true`, files without frontmatter are included in the scan. They will violate any `required` rules that match their path.

---

## Output

```rust
pub struct CheckResult {
    pub files_checked: usize,
    pub field_violations: Vec<FieldViolation>,
    pub new_fields: Vec<NewField>,              // see shared.md
}

/// A single rule violation for a field, with all offending files.
pub struct FieldViolation {
    pub field: String,
    pub kind: ViolationKind,       // see shared.md
    pub rule: String,              // the toml rule (e.g. "required in [\"blog/**\"]")
    pub files: Vec<ViolatingFile>,
}

pub struct ViolatingFile {
    pub path: PathBuf,
    pub detail: Option<String>,    // e.g. "got Integer" for WrongType, None for others
}
```

A single field can appear in multiple `FieldViolation` entries if it violates different rules (e.g. wrong type in some files AND missing in others).

### Human format (clean)

```
Checked 5 files — no violations
```

### Human format (violations)

```
title: required in ["blog/**"]
  missing: blog/post.md, blog/draft.md

draft: allowed in ["blog/**"]
  disallowed: notes/idea.md, notes/todo.md

tags: type String[]
  wrong type: blog/post.md (got String), blog/other.md (got Integer)

Checked 5 files — 3 field violations
```

### Human format (new fields)

Appended after the violation summary:

```
New fields (not in mdvs.toml):
  category (2 files)
  author (1 file)
Run 'mdvs update' to incorporate new fields.
```

---

## Violations

| Kind             | Condition                                                    | Output example                                    |
|------------------|--------------------------------------------------------------|---------------------------------------------------|
| MissingRequired  | File matches a `required` glob but field is absent           | `title: required in ["blog/**"]`                  |
| WrongType        | Value type doesn't match declared type                       | `tags: type String[]`                             |
| Disallowed       | File has the field but path doesn't match any `allowed` glob | `draft: allowed in ["blog/**"]`                   |

**Type leniency:** an integer value in a float field is not a violation.

---

## Prerequisites

Requires a valid `mdvs.toml`. If missing or invalid:

| Condition               | Message                                                                       |
|-------------------------|-------------------------------------------------------------------------------|
| No `mdvs.toml`         | `no mdvs.toml found in '<path>' — run 'mdvs init' to set up`                 |
| Invalid/incomplete toml | `mdvs.toml is invalid: <details> — fix the file or run 'mdvs init --force'`  |

This prerequisite applies to all commands except `init`.

---

## Examples

```bash
# Check current directory
mdvs check

# Check a specific directory
mdvs check ~/notes
```
