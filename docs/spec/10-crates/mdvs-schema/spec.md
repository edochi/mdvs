# Crate: `mdvs-schema`

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Configuration](../../40-configuration/frontmatter-toml.md)

---

## Overview

Shared library crate containing field definitions, the type system, TOML parsing, field discovery, tree inference, and lock file types. Dependency of both `mfv` and `mdvs`. Has no knowledge of DataFusion, embeddings, or search.

**Responsibilities:**

- Parse `mfv.toml` / `mdvs.toml` into structured field definitions
- Define the field type system (`string`, `string[]`, `date`, `boolean`, `integer`, `float`, `enum`)
- Infer field types from observed YAML/TOML/JSON values
- Provide path-scoped validation via `allowed` and `required` glob patterns
- Infer `allowed`/`required` patterns from file observations (tree inference)
- Define lock file types for capturing discovery snapshots

**Not responsible for:**

- Frontmatter extraction from files (gray_matter, used by consumers)
- Actual validation execution (mfv crate)
- Database schema generation or embeddings (mdvs crate)

---

## Public Types

### `FieldType`

```rust
enum FieldType {
    String,
    StringArray,
    Date,
    Boolean,
    Integer,
    Float,
    Enum,
}
```

### `FieldDef`

A single field definition as parsed from the TOML config.

```rust
struct FieldDef {
    /// Field name as it appears in frontmatter
    name: String,
    /// Expected value type
    field_type: FieldType,
    /// Glob patterns where this field may appear
    /// `[]` = nowhere, `["**"]` = everywhere
    allowed: Vec<String>,
    /// Glob patterns where this field must be present
    /// `[]` = not required anywhere, `["**"]` = required everywhere
    required: Vec<String>,
    /// Regex pattern the value must match (string/date fields)
    pattern: Option<String>,
    /// Allowed values (enum fields)
    values: Vec<String>,
}
```

#### Methods

```rust
impl FieldDef {
    /// Check if this field is allowed at a given relative file path.
    /// Returns true if any `allowed` pattern matches the path.
    /// Returns false if `allowed` is empty.
    fn is_allowed_at(&self, path: &str) -> bool

    /// Check if this field is required at a given relative file path.
    /// Returns true if any `required` pattern matches the path.
    /// Returns false if `required` is empty.
    fn is_required_at(&self, path: &str) -> bool
}
```

#### TOML defaults

When parsing from TOML, `allowed` defaults to `["**"]` (field allowed everywhere) and `required` defaults to `[]` (field not required anywhere). This makes the common case minimal:

```toml
[[fields.field]]
name = "title"
type = "string"
# allowed defaults to ["**"], required defaults to []
```

#### Invariant: `required ⊆ allowed`

A field cannot be required somewhere it isn't allowed. If `required` is non-empty but `allowed` is empty, schema validation fails.

### `Schema`

The complete parsed config file.

```rust
struct Schema {
    /// File glob for discovery (from `[directory].glob`)
    glob: String,
    /// Field definitions (from `[[fields.field]]`)
    fields: Vec<FieldDef>,
}
```

#### Methods

```rust
impl Schema {
    /// Load a schema from a file path.
    fn from_file(path: &Path) -> Result<Schema, SchemaError>

    /// Return field definitions that are allowed at a given relative file path.
    fn rules_for_path(&self, rel_path: &str) -> Vec<&FieldDef>

    /// Generate TOML string representation.
    fn to_toml_string(&self) -> String
}

impl FromStr for Schema {
    /// Parse from TOML string. Validates field definitions.
    fn from_str(s: &str) -> Result<Schema, SchemaError>
}
```

#### Validation

`from_str` validates:

- No duplicate field names
- `values` is only set when `type = "enum"`
- `pattern` is a valid regex (only for `string`/`date` types)
- `allowed` and `required` are valid glob patterns
- `required ⊆ allowed` (required non-empty implies allowed non-empty)
- Unknown top-level sections are silently ignored (allows `mfv.toml` and `mdvs.toml` to share format)

### `SchemaError`

```rust
enum SchemaError {
    Io(std::io::Error),
    Parse(toml::de::Error),
    Validation(String),
    Glob(globset::Error),
}
```

### `FieldInfo`

A field discovered by scanning frontmatter across files.

```rust
struct FieldInfo {
    /// Field name as it appears in frontmatter
    name: String,
    /// Inferred type based on the most common value type seen
    field_type: FieldType,
    /// Relative paths of files containing this field
    files: Vec<String>,
}
```

### `FieldPaths`

Inferred `allowed` and `required` patterns for a single field. Output of tree inference.

```rust
struct FieldPaths {
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}
```

### Lock File Types

```rust
struct LockFile {
    discovery: LockDiscovery,
    fields: Vec<LockField>,
}

struct LockDiscovery {
    /// Total markdown files matched by the glob
    total_files: usize,
    /// Files that had parseable frontmatter
    files_with_frontmatter: usize,
    /// Glob pattern used for file matching
    glob: String,
    /// ISO 8601 timestamp of when the lock was generated
    generated_at: String,
}

struct LockField {
    /// Field name as it appears in frontmatter
    name: String,
    /// Inferred type
    field_type: FieldType,
    /// Relative paths of files containing this field
    files: Vec<String>,
}
```

```rust
impl LockFile {
    /// Build from discovery results.
    fn from_discovery(
        fields: &[FieldInfo],
        total_files: usize,
        files_with_frontmatter: usize,
        glob: &str,
        generated_at: &str,
    ) -> Self

    /// Serialize to TOML string.
    fn to_toml_string(&self) -> String
}
```

---

## Type Inference

When `type` is not explicitly set in TOML, the type is inferred from observed values during `mfv init`. The inference logic lives in this crate so both tools use the same rules.

### `infer_type` Function

```rust
fn infer_type(value: &serde_json::Value) -> FieldType
```

Takes a single JSON value (converted from YAML/TOML frontmatter by `gray_matter`). Returns the inferred `FieldType`.

| Value | Inferred Type |
|---|---|
| JSON boolean | `Boolean` |
| JSON integer | `Integer` |
| JSON float | `Float` |
| JSON string matching YYYY-MM-DD | `Date` |
| JSON string (other) | `String` |
| JSON array | `StringArray` |
| Anything else | `String` |

**Mixed types:** When a field has different types across files, `discover_fields` picks the most common type. The user can override with an explicit `type` in the config.

---

## Frontmatter Discovery

### `discover_fields` Function

```rust
fn discover_fields(
    file_frontmatters: &[(&str, Option<&serde_json::Value>)]
) -> Vec<FieldInfo>
```

Takes `(relative_path, frontmatter)` pairs. For each field found across all files, tracks:
- The most common inferred type (majority vote)
- Which files contain the field

Returns `Vec<FieldInfo>` sorted by frequency (descending), then name (ascending).

---

## Tree Inference

### `infer_field_paths` Function

```rust
fn infer_field_paths(
    observations: &[(PathBuf, HashSet<String>)]
) -> BTreeMap<String, FieldPaths>
```

Given a flat list of `(file_path, set_of_fields)`, infers `allowed` and `required` glob patterns for each field by building a directory tree and walking it.

See [Workflow: Inference](../../30-workflows/inference.md) for the full algorithm specification.

Key behaviors:
- Leaf nodes (direct files) emit `*` (shallow) patterns
- Directory nodes emit `**` (recursive) patterns via collapse
- Collapse upgrades `*` to `**` when a directory confirms the claim
- `required` only comes from directory-level `all` sets (not leaf initialization)

---

## Dependencies

| Crate | Purpose |
|---|---|
| `serde` + `toml` | Parse TOML config |
| `serde_json` | Type inference from JSON values (via gray_matter) |
| `regex` | Compile and validate `pattern` rules |
| `globset` | Compile and validate `allowed`/`required` globs; path matching |
| `indextree` | Arena-backed tree for inference algorithm |
| `chrono` | Date string validation |

---

## Related Documents

- [Terminology](../../01-terminology.md) — canonical definitions for field, field type
- [Configuration](../../40-configuration/frontmatter-toml.md) — file format this crate parses
- [Workflow: Inference](../../30-workflows/inference.md) — tree inference algorithm
- [Crate: mfv](../mfv/spec.md) — validation engine that consumes this crate
- [Crate: mdvs](../mdvs/spec.md) — search tool that consumes this crate
