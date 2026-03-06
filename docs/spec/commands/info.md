# `mdvs info`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md)

---

## Synopsis

```
mdvs info [path]
```

| Flag   | Type       | Default | Description                    |
|--------|------------|---------|--------------------------------|
| `path` | positional | `.`     | Directory containing mdvs.toml |

---

## Behavior

1. Read `mdvs.toml` (see [Prerequisites](check.md#prerequisites))
2. Scan markdown files using `[scan]` config (count only)
3. If `.mdvs/` exists: read parquet metadata and file/chunk counts
4. Collect `InfoResult`
5. Print result

Read-only — no side effects.

---

## Output

```rust
#[derive(Serialize)]
pub struct InfoResult {
    pub scan_glob: String,
    pub files_on_disk: usize,
    pub fields: Vec<DiscoveredField>,       // from toml schema
    pub ignored_fields: Vec<String>,
    pub index: Option<IndexInfo>,
}

#[derive(Serialize)]
pub struct IndexInfo {
    pub model: String,
    pub model_revision: Option<String>,
    pub chunk_size: usize,
    pub files_indexed: usize,
    pub chunks: usize,
    pub built_at: String,                   // ISO 8601
    pub config_match: bool,                 // toml still matches parquet metadata
}
```

### Human format (with index)

```
Scan: glob = "**", 15 files on disk

Fields:
  title   String   (required in ["**"])
  tags    String[] (allowed in ["blog/**"])
  draft   Boolean
Ignored: internal_id, notes

Index:
  Model: minishlab/potion-base-8M (rev abc123)
  Chunk size: 1024
  12 files indexed, 47 chunks
  Built: 2026-03-01T14:30:00Z
  Status: 3 files not indexed
```

### Human format (no index)

```
Scan: glob = "**", 15 files on disk

Fields:
  title   String   (required in ["**"])
  tags    String[] (allowed in ["blog/**"])
Ignored: internal_id, notes

No index built — run 'mdvs build' to create one
```

### Human format (config mismatch)

When `config_match` is false, the status line shows what changed:

```
  Status: config changed — rebuild recommended
```

---

## Errors

See [Prerequisites](check.md#prerequisites) for toml validation errors.

---

## Examples

```bash
# Show info for current directory
mdvs info

# Show info for a specific directory
mdvs info ~/notes

# JSON output
mdvs info --output json
```
