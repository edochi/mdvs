# Crate: `mfv` (Markdown Frontmatter Validator)

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Crate: mdvs-schema](../mdvs-schema/spec.md) | [Configuration](../../40-configuration/frontmatter-toml.md)

---

## Overview

Standalone frontmatter validation library and CLI binary (~2MB). No DuckDB, no embeddings, no search. Independently publishable on crates.io as `mfv`.

**Target users:** bloggers, documentation maintainers, CI pipelines — anyone with markdown + frontmatter who wants linting.

**Responsibilities:**

- Scan markdown files and extract frontmatter via `gray_matter`
- Validate frontmatter against a field schema (`mfv.toml` / `mdvs.toml`)
- Generate a field schema by scanning, inferring types, and inferring allowed/required patterns
- Report diagnostics in human-readable, JSON, or GitHub Actions format
- Provide a library API (`mfv::validate`) consumed by `mdvs`

**Not responsible for:**

- Field type definitions or TOML parsing (delegated to `mdvs-schema`)
- Database schema or embeddings

---

## CLI

```
mfv <command> [options]

COMMANDS:
    init      Scan frontmatter, generate config + lock file
    update    Refresh lock file by re-scanning markdown files
    check     Validate files against schema
```

### `mfv init`

```
mfv init [--dir <path>] [--glob <pattern>] [--config <path>] [--force] [--dry-run]
```

Scans markdown files, discovers frontmatter fields, infers types and allowed/required patterns via tree inference, and writes `mfv.toml` (schema) and `mfv.lock` (per-file observations).

**Flags:**

| Flag | Default | Description |
|---|---|---|
| `--dir <path>` | `.` | Directory to scan |
| `--glob <pattern>` | `**` | File matching glob |
| `--config <path>` | `mfv.toml` | Output config file path |
| `--force` | off | Overwrite existing config and lock |
| `--dry-run` | off | Print discovery table only, write nothing |

**Flow:**

1. Walk directory with glob filter
2. Extract frontmatter from each file via `gray_matter`
3. Discover fields and infer types via `mdvs_schema::discover_fields`
4. Build per-file field observations and run `mdvs_schema::infer_field_paths` (tree inference)
5. Combine: `FieldDef` = inferred type + inferred `allowed`/`required` patterns
6. Display frequency table to stderr
7. Write `mfv.toml` (schema with patterns) and `mfv.lock` (per-file observations)

**If config already exists:** Error with exit 2, suggests `--force`. With `--force`, overwrites both files.

### `mfv update`

```
mfv update [--dir <path>] [--config <path>]
```

Re-scans the directory, discovers fields, and refreshes the lock file. Does not modify config. Analogous to `cargo update`.

**Flags:**

| Flag | Default | Description |
|---|---|---|
| `--dir <path>` | `.` | Directory to scan |
| `--config <path>` | auto | Path to config file (auto-discover `mfv.toml` / `mdvs.toml`) |

**Flow:**

1. Find existing config (`--config` or auto-discover `mfv.toml` → `mdvs.toml`)
2. Load config to extract glob pattern
3. Scan directory, discover fields, infer patterns (identical to init steps 1-4)
4. Display frequency table to stderr
5. Write lock file (overwrites existing)

**Exit codes:** 0 = success, 2 = config/IO error (missing config, bad directory, etc.)

### `mfv check`

```
mfv check [--dir <path>] [--schema <path>] [--format <fmt>]

Options:
    --dir       Directory to scan (default: .)
    --schema    Path to schema file (default: auto-discover)
    --format    Output format: human (default), json, github
```

Validates all matching markdown files against the field schema.

**Config discovery** (when `--schema` is not given): `mfv.toml` → `mdvs.toml` → error.

**Exit codes:**

| Code | Meaning |
|---|---|
| 0 | All files valid |
| 1 | Validation errors found |
| 2 | Schema/config error (bad TOML, missing file, directory not found, etc.) |

---

## Library API

The `mfv` crate exposes a library API so `mdvs` can delegate validation without spawning a subprocess.

### Modules

- `scan` — file discovery and frontmatter extraction
- `validate` — validation logic
- `diagnostic` — diagnostic types
- `output` — output formatting (human, JSON, GitHub Actions)

### `validate`

```rust
pub fn validate(
    files: &[ScannedFile],
    schema: &Schema,
) -> Vec<Diagnostic>
```

Validates scanned files against a schema. Returns a list of diagnostics (empty = all valid).

### `ScannedFile`

```rust
pub struct ScannedFile {
    pub rel_path: String,
    pub frontmatter: Option<serde_json::Value>,
}
```

### `Diagnostic`

```rust
pub struct Diagnostic {
    /// Relative path of the file
    pub file: String,
    /// Field name that has the problem
    pub field: String,
    /// What's wrong
    pub kind: DiagnosticKind,
}

pub enum DiagnosticKind {
    /// Required field is missing
    MissingRequired,
    /// Value has the wrong type
    WrongType { expected: String, got: String },
    /// Value doesn't match regex pattern
    PatternMismatch { pattern: String, value: String },
    /// Value not in allowed enum values
    InvalidEnum { value: String, allowed: Vec<String> },
    /// Field is present but not allowed at this file's path
    NotAllowed,
}
```

---

## Validation Rules

All rules are defined in the TOML config via `mdvs-schema`. The `mfv` crate executes them.

### Rule Evaluation Order

For each file:

1. Determine which fields apply to this file via `schema.rules_for_path(rel_path)` — filters by `allowed` patterns.
2. For each applicable field:
   a. If `rule.is_required_at(rel_path)` and field is absent → `MissingRequired`.
   b. If field is present, check type compatibility → `WrongType` if wrong.
   c. If `pattern` is set and value is a string, check regex → `PatternMismatch`.
   d. If `values` is set (enum), check membership → `InvalidEnum`.
3. **Allowed enforcement:** For each key in the file's frontmatter, check that some schema field with that name has `is_allowed_at(rel_path)`. If no match → `NotAllowed`. This catches both fields with restricted `allowed` patterns appearing outside their scope and fields not defined in the schema at all.

### Path-Scoped Rules

The `allowed` and `required` patterns scope when a field's rules apply.

```toml
[[fields.field]]
name = "status"
type = "string"
allowed = ["blog/**"]
required = ["blog/**"]
```

A file at `blog/my-post.md` must have `status` (it's both allowed and required there). A file at `notes/random.md` is not checked for `status` at all (not in `allowed`).

---

## Output Formats

### Human (default)

```
  blog/half-finished-post.md: field 'status': required field missing
  papers/new-idea.md: field 'doi': value "not-a-doi" does not match pattern /^10\.\d{4,9}/.*/
  notes/quick-thought.md: field 'tags': expected type 'string[]', got 'string'
```

### JSON

```json
[
  {
    "file": "blog/half-finished-post.md",
    "field": "status",
    "message": "required field missing"
  }
]
```

Empty array `[]` when all files are valid.

### GitHub Actions (`--format github`)

```
::error file=blog/half-finished-post.md::field 'status': required field missing
```

Enables inline annotations in GitHub PR diffs.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `mdvs-schema` | Field definitions, type system, TOML parsing, discovery, inference |
| `gray_matter` | Frontmatter extraction from markdown files |
| `glob` | Filesystem traversal with glob patterns |
| `clap` | CLI argument parsing |
| `anyhow` | Error handling |
| `serde_json` | JSON output format |
| `chrono` | Timestamp generation for lock file |

---

## Related Documents

- [Terminology](../../01-terminology.md) — canonical definitions for frontmatter, field, field type
- [Crate: mdvs-schema](../mdvs-schema/spec.md) — types and parsing consumed by this crate
- [Configuration](../../40-configuration/frontmatter-toml.md) — schema file format
- [Workflow: Init](../../30-workflows/init.md) — init flow
- [Workflow: Inference](../../30-workflows/inference.md) — tree inference algorithm
