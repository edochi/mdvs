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
