# Crate: `mdvs-schema`

**Status: DRAFT**

**Cross-references:** [Terminology](../../01-terminology.md) | [Configuration: frontmatter.toml](../../40-configuration/frontmatter-toml.md)

---

## Overview

Shared library crate containing field definitions, the type system, and TOML parsing for `frontmatter.toml`. Dependency of both `mfv` and `mdvs`. Has no knowledge of DuckDB, embeddings, or search.

**Responsibilities:**

- Parse `frontmatter.toml` into structured field definitions
- Define the field type system (`string`, `string[]`, `date`, `boolean`, `integer`, `float`, `enum`)
- Infer field types from observed YAML values
- Provide validation rule types (`required`, `paths`, `pattern`, `values`)
- Expose the `promoted` flag without interpreting it (mdvs-specific concern)

**Not responsible for:**

- Database schema generation (mdvs crate)
- Frontmatter extraction from files (gray_matter, used by consumers)
- Actual validation execution (mfv crate)

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

A single field definition as parsed from `frontmatter.toml`.

```rust
struct FieldDef {
    /// Field name (the TOML key under `[fields.*]`)
    name: String,
    /// Explicit type, or inferred from values
    field_type: FieldType,
    /// Whether the type was explicitly set or inferred
    type_source: TypeSource,
    /// Validation: field must be present
    required: bool,
    /// Validation: glob patterns where rules apply (empty = all files)
    paths: Vec<String>,
    /// Validation: regex the value must match (strings only)
    pattern: Option<String>,
    /// Validation: allowed values (enum type)
    values: Vec<String>,
    /// mdvs-specific: becomes a SQL column vs. JSON metadata
    promoted: bool,
}
```

### `TypeSource`

```rust
enum TypeSource {
    /// User explicitly set `type = "..."` in frontmatter.toml
    Explicit,
    /// Inferred from observed values during init
    Inferred,
}
```

### `Schema`

The complete parsed `frontmatter.toml`.

```rust
struct Schema {
    /// File glob for discovery (from `[directory].glob`)
    glob: String,
    /// Field definitions (from `[fields.*]`)
    fields: Vec<FieldDef>,
}
```

---

## Type Inference

When `type` is not explicitly set in `frontmatter.toml`, the type is inferred from observed YAML values during `mfv init` or `mdvs init`. The inference logic lives in this crate so both tools use the same rules.

### Inference Rules

Evaluated in order against observed values for a field across scanned files:

| Priority | Condition | Inferred Type |
|---|---|---|
| 1 | All values are YAML sequences (lists) | `StringArray` |
| 2 | All values parse as dates (YYYY-MM-DD or similar) | `Date` |
| 3 | All values are YAML booleans (`true`/`false`) | `Boolean` |
| 4 | All values are YAML integers | `Integer` |
| 5 | All values are YAML floats | `Float` |
| 6 | Fallback | `String` |

**Mixed types:** If values for a field have mixed types across files (e.g., some strings, some lists), the field falls back to `String`. The user can override with an explicit `type` in `frontmatter.toml`.

### `infer_type` Function

```rust
fn infer_type(values: &[serde_yaml::Value]) -> FieldType
```

Takes a slice of observed YAML values for a single field across multiple files. Returns the inferred `FieldType`.

---

## TOML Parsing

### `Schema::from_file`

```rust
impl Schema {
    fn from_file(path: &Path) -> Result<Schema>
    fn from_str(toml: &str) -> Result<Schema>
}
```

Parses `frontmatter.toml` into a `Schema`. Validates:

- All field names are valid identifiers
- Explicit types are recognized values
- `values` is only set when `type = "enum"`
- `pattern` is a valid regex
- `paths` entries are valid glob patterns

Returns an error with a clear message on invalid config (exit code 2 in CLI tools).

### Schema Accessors

```rust
impl Schema {
    /// All fields with `promoted = true`
    fn promoted_fields(&self) -> Vec<&FieldDef>

    /// All fields (promoted and non-promoted)
    fn all_fields(&self) -> &[FieldDef]

    /// Lookup a field by name
    fn field(&self, name: &str) -> Option<&FieldDef>

    /// Fields with validation rules applicable to a given file path
    fn rules_for_path(&self, path: &str) -> Vec<&FieldDef>
}
```

---

## DuckDB Type Mapping

This crate defines the mapping from `FieldType` to DuckDB column types, even though it doesn't depend on DuckDB. The mapping is used by `mdvs` when generating `CREATE TABLE` statements.

| `FieldType` | DuckDB Column Type |
|---|---|
| `String` | `VARCHAR` |
| `StringArray` | `VARCHAR[]` |
| `Date` | `DATE` |
| `Boolean` | `BOOLEAN` |
| `Integer` | `BIGINT` |
| `Float` | `DOUBLE` |
| `Enum` | `VARCHAR` |

`Enum` maps to `VARCHAR` because DuckDB doesn't enforce enum constraints on insert. Enum validation is application-level (via `mfv check` / `mdvs validate`).

```rust
impl FieldType {
    fn duckdb_type(&self) -> &'static str
}
```

---

## Frontmatter Discovery

Scanning logic used by both `mfv init` and `mdvs init` to discover fields across a vault.

### `discover_fields` Function

```rust
struct FieldStats {
    name: String,
    file_count: usize,
    inferred_type: FieldType,
    sample_values: Vec<String>,
}

fn discover_fields(
    files: &[(PathBuf, serde_yaml::Mapping)],
) -> Vec<FieldStats>
```

Takes parsed frontmatter from scanned files. Returns per-field statistics sorted by frequency (descending). Used to populate the interactive field selection prompt and the `mfv inspect` output.

---

## Dependencies

| Crate | Purpose |
|---|---|
| `serde` + `toml` | Parse `frontmatter.toml` |
| `serde_yaml` | Type inference from YAML values |
| `regex` | Compile and validate `pattern` rules |
| `globset` | Compile and validate `paths` globs |

---

## Related Documents

- [Terminology](../../01-terminology.md) — canonical definitions for field, promoted field, field type
- [Configuration: frontmatter.toml](../../40-configuration/frontmatter-toml.md) — file format this crate parses
- [Crate: mfv](../mfv/spec.md) — validation engine that consumes this crate
- [Crate: mdvs](../mdvs/spec.md) — search tool that consumes this crate
