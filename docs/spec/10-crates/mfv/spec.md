# Crate: `mfv` (Markdown Frontmatter Validator)

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Crate: mdvs-schema](../mdvs-schema/spec.md) | [Configuration: frontmatter.toml](../../40-configuration/frontmatter-toml.md)

---

## Overview

Standalone frontmatter validation library and CLI binary (~2MB). No DuckDB, no embeddings, no search. Independently publishable on crates.io as `mfv`.

**Target users:** bloggers, documentation maintainers, CI pipelines — anyone with markdown + frontmatter who wants linting.

**Responsibilities:**

- Scan markdown files and extract frontmatter via `gray_matter`
- Validate frontmatter against a field schema (`frontmatter.toml`)
- Generate a field schema by scanning and inferring types
- Report diagnostics in human-readable, JSON, or GitHub Actions format
- Provide a library API (`mfv::validate`) consumed by `mdvs validate`

**Not responsible for:**

- Field type definitions or TOML parsing (delegated to `mdvs-schema`)
- The `promoted` flag (ignored — that's an mdvs concern)
- Database schema or embeddings

---

## CLI

```
mfv <command> [options]

COMMANDS:
    init      Scan frontmatter, generate schema file
    check     Validate files against schema
    inspect   Show frontmatter stats
```

### `mfv init`

```
mfv init [--dir <path>] [--glob <pattern>]
```

Scans markdown files, discovers frontmatter fields, and generates `frontmatter.toml` with inferred types and no validation rules. The user adds rules by editing the file afterward.

**Behavior:**

1. Walk directory with glob filter (default: `**/*.md`)
2. Extract frontmatter from each file via `gray_matter`
3. Collect field statistics via `mdvs_schema::discover_fields`
4. Display the interactive frequency table
5. Write `frontmatter.toml` with inferred types, no validation rules, no `promoted` flags

**Difference from `mdvs init`:** No promoted field selection (that concept belongs to mdvs). No `.mdvs.toml` generation. No model download.

**If `frontmatter.toml` already exists:** Prompt for confirmation before overwriting. `--force` skips the prompt.

### `mfv check`

```
mfv check [--dir <path>] [--schema <path>] [--format <fmt>]

Options:
    --dir       Directory to scan (default: .)
    --schema    Path to frontmatter.toml (default: ./frontmatter.toml)
    --format    Output format: human (default), json, github
```

Validates all matching markdown files against the field schema.

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | All files valid |
| 1 | Validation errors found |
| 2 | Schema/config error (bad frontmatter.toml, missing file, etc.) |

### `mfv inspect`

```
mfv inspect [--dir <path>]
```

Read-only discovery. Shows field frequency, inferred types, and sample values. Same data as `init` but does not write any files.

---

## Library API

The `mfv` crate exposes a library API so `mdvs validate` can delegate without spawning a subprocess.

### `validate`

```rust
pub fn validate(
    dir: &Path,
    schema: &Schema,
) -> Result<Vec<Diagnostic>>
```

Scans all matching files in `dir`, validates each against `schema`, returns a list of diagnostics. Empty list means all valid.

### `Diagnostic`

```rust
pub struct Diagnostic {
    /// Relative path to the file with the error
    pub file: PathBuf,
    /// The field that failed validation (if applicable)
    pub field: Option<String>,
    /// What went wrong
    pub kind: DiagnosticKind,
    /// Human-readable error message
    pub message: String,
}

pub enum DiagnosticKind {
    /// Required field is missing
    MissingRequired,
    /// Value doesn't match expected type
    TypeMismatch {
        expected: FieldType,
        actual: String,
    },
    /// Value doesn't match regex pattern
    PatternMismatch {
        pattern: String,
        value: String,
    },
    /// Value not in allowed enum values
    InvalidEnumValue {
        allowed: Vec<String>,
        actual: String,
    },
    /// Frontmatter parse error (malformed YAML, etc.)
    ParseError,
}
```

---

## Validation Rules

All rules are defined in `frontmatter.toml` via `mdvs-schema`. The `mfv` crate executes them.

### Rule Evaluation Order

For each file:

1. Parse frontmatter via `gray_matter`. If parsing fails → `ParseError` diagnostic, skip remaining rules for this file.
2. For each field in the schema:
   a. Check if the field's `paths` globs match this file. If `paths` is empty, the rule applies to all files.
   b. If `required = true` and field is absent → `MissingRequired`.
   c. If field is present, check type compatibility → `TypeMismatch` if wrong.
   d. If `pattern` is set and field is a string, check regex → `PatternMismatch`.
   e. If `values` is set (enum), check membership → `InvalidEnumValue`.

### Path-Scoped Rules

The `paths` field in a field definition scopes when that field's rules apply. Paths are glob patterns relative to the vault root.

```toml
[fields.status]
required = true
paths = ["blog/**"]  # only required in blog/ subtree
```

A file at `blog/my-post.md` must have `status`. A file at `notes/random.md` is not checked for `status`.

If `paths` is omitted or empty, the rule applies to all files.

---

## Output Formats

### Human (default)

```
Checking 1203 files against frontmatter.toml...

  ✗ blog/half-finished-post.md
      missing required field 'status' (required in blog/**)

  ✗ papers/new-idea.md
      field 'doi' value "not-a-doi" doesn't match pattern ^10\.\d{4,9}/.*

  ✗ notes/quick-thought.md
      field 'tags' expected string[], got string

3 errors in 1203 files.
```

### JSON

```json
{
  "total_files": 1203,
  "errors": [
    {
      "file": "blog/half-finished-post.md",
      "field": "status",
      "kind": "missing_required",
      "message": "missing required field 'status' (required in blog/**)"
    }
  ]
}
```

### GitHub Actions (`--format github`)

```
::error file=blog/half-finished-post.md::missing required field 'status' (required in blog/**)
::error file=papers/new-idea.md::field 'doi' value "not-a-doi" doesn't match pattern ^10\.\d{4,9}/.*
```

Enables inline annotations in GitHub PR diffs.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `mdvs-schema` | Field definitions, type system, TOML parsing |
| `gray_matter` | Frontmatter extraction from markdown files |
| `walkdir` | Filesystem traversal |
| `globset` | Path matching for `paths` rules |
| `clap` | CLI argument parsing |
| `anyhow` | Error handling |
| `serde_json` | JSON output format |

---

## Related Documents

- [Terminology](../../01-terminology.md) — canonical definitions for frontmatter, field, field schema
- [Crate: mdvs-schema](../mdvs-schema/spec.md) — types and parsing consumed by this crate
- [Crate: mdvs](../mdvs/spec.md) — consumes `mfv::validate` for its `validate` command
- [Configuration: frontmatter.toml](../../40-configuration/frontmatter-toml.md) — schema format
- [Workflow: Init](../../30-workflows/init.md) — init flow for both `mfv` and `mdvs`
