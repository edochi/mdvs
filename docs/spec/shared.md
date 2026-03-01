# Shared Types

**Status: DRAFT**

Output structs shared across multiple commands. Each command collects its results
into a struct before display — these are the types that appear in more than one command.

---

## DiscoveredField

Represents a single frontmatter field found during scanning.

**Used by:** init, update, info

| Field       | Type   | Description                              |
|-------------|--------|------------------------------------------|
| name        | String | Field name (e.g. "title", "tags")        |
| field_type  | String | Inferred type (e.g. "String", "Boolean") |
| files_found | usize  | Number of files containing this field    |
| total_files | usize  | Total files scanned (for "N/M" display)  |

---

## FieldViolation

A single rule violation for a field, grouped with all offending files.

**Used by:** check, update

| Field  | Type              | Description                                              |
|--------|-------------------|----------------------------------------------------------|
| field  | String            | Field name                                               |
| kind   | ViolationKind     | Type of violation                                        |
| rule   | String            | The toml rule (e.g. `required in ["blog/**"]`)           |
| files  | Vec\<ViolatingFile\> | Files that violate this rule                          |

A single field can appear in multiple `FieldViolation` entries if it violates different rules.

### ViolationKind

| Variant          | Meaning                                                        |
|------------------|----------------------------------------------------------------|
| MissingRequired  | File matches a `required` glob but doesn't have the field      |
| WrongType        | Field value doesn't match declared type (int-in-float lenient) |
| Disallowed       | File has the field but doesn't match any `allowed` glob        |

### ViolatingFile

| Field  | Type            | Description                                          |
|--------|-----------------|------------------------------------------------------|
| path   | PathBuf         | File path                                            |
| detail | Option\<String\> | Extra info (e.g. "got Integer" for WrongType)       |

---

## NewField

A frontmatter field found in files but not present in `mdvs.toml` (neither in `[[fields.field]]` nor in `[fields].ignore`).

**Used by:** check, update

| Field       | Type   | Description                           |
|-------------|--------|---------------------------------------|
| name        | String | Field name                            |
| files_found | usize  | Number of files containing this field |
