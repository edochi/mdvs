# Shared Types

Output and validation types used across commands. All defined in `src/output.rs` unless noted.

## Output Format

```rust
pub enum OutputFormat { Text, Json }  // output.rs:7
```

Global `--output`/`-o` flag. Default `Text`. JSON is free via `#[derive(Serialize)]` on all outcome structs.

## Field Hints

```rust
pub enum FieldHint {                   // output.rs:16
    EscapeSingleQuotes,                // field name contains '
    EscapeDoubleQuotes,                // field name contains "
    ContainsSpaces,                    // field name contains spaces
}
```

`field_hints(name)` at `output.rs:39` detects special characters in field names and suggests escaping for `--where` queries. Used in `info` and `check` output.

## Discovered Field

```rust
pub struct DiscoveredField {           // output.rs:64
    pub name: String,
    pub field_type: String,            // display form: "String", "Integer[]", etc.
    pub files_found: usize,
    pub total_files: usize,
    pub allowed: Option<Vec<String>>,  // verbose only
    pub required: Option<Vec<String>>, // verbose only
    pub nullable: bool,
    pub hints: Vec<FieldHint>,
}
```

Used in `InitOutcome.fields` and `UpdateOutcome.added`.

## Changed Field

```rust
pub struct ChangedField {              // output.rs:88
    pub name: String,
    pub changes: Vec<FieldChange>,
}

pub enum FieldChange {                 // output.rs:98
    Type { old: String, new: String },
    Allowed { old: Vec<String>, new: Vec<String> },
    Required { old: Vec<String>, new: Vec<String> },
    Nullable { old: bool, new: bool },
}
```

Used in `UpdateOutcome.changed`. Each variant carries old and new values.

## Removed Field

```rust
pub struct RemovedField {              // output.rs:163
    pub name: String,
    pub allowed: Option<Vec<String>>,  // previous allowed globs (verbose only)
}
```

Used in `UpdateOutcome.removed`.

## Violations

```rust
pub enum ViolationKind {               // output.rs:173
    MissingRequired,
    WrongType,
    Disallowed,
    NullNotAllowed,
    InvalidCategory,
}

pub struct ViolatingFile {             // output.rs:188
    pub path: PathBuf,
    pub detail: Option<String>,        // e.g., "got String", "got \"pending\""
}

pub struct FieldViolation {            // output.rs:197
    pub field: String,
    pub kind: ViolationKind,
    pub rule: String,                  // e.g., "type Integer", "categories = [...]"
    pub files: Vec<ViolatingFile>,
}
```

Used in `CheckOutcome.violations` and `ValidateOutcome.violations`.

## New Field

```rust
pub struct NewField {                  // output.rs:210
    pub name: String,
    pub files: Vec<PathBuf>,
}
```

Informational — fields in frontmatter but not in `mdvs.toml`. Does not affect exit code.

## Constraint Violation

```rust
pub(crate) struct ConstraintViolation { // schema/constraints/mod.rs:52
    pub rule: String,                   // "categories = [\"draft\", \"published\"]"
    pub detail: String,                 // "got \"pending\""
}
```

Internal type — mapped to `ViolationKind::InvalidCategory` + `ViolatingFile::detail` in the check pipeline.

## Build File Detail

```rust
pub struct BuildFileDetail {           // output.rs:222
    pub filepath: String,
    pub chunks: usize,
}
```

Used in `BuildOutcome.file_details` for verbose build output.
