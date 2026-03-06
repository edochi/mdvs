# `mdvs update`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md), [check](check.md), [build](build.md)

---

## Synopsis

```
mdvs update [path] [flags]
```

| Flag              | Type       | Default     | Description                                       |
|-------------------|------------|-------------|---------------------------------------------------|
| `path`            | positional | `.`         | Directory containing mdvs.toml                    |
| `--reinfer`       | string[]   | (none)      | Re-infer specific field(s) — can be repeated      |
| `--reinfer-all`   | bool       | false       | Re-infer all fields                                |
| `--build`         | bool       | (from toml) | Override `auto_build` from `[update]`             |
| `--dry-run`       | bool       | false       | Show what would change, write nothing             |

---

## Behavior

1. Read `mdvs.toml` (see [Prerequisites](check.md#prerequisites))
2. Scan all markdown files using `[scan]` config
3. Determine inference mode:
   - **Default**: infer only new fields (not in `[[fields.field]]`, not in `[fields].ignore`)
   - **`--reinfer <field>`**: remove named field(s) from toml, re-infer from scan data
   - **`--reinfer-all`**: remove all `[[fields.field]]` entries, re-infer everything
4. Run inference on target fields
5. Collect `UpdateResult`
6. If `--dry-run`: print result, return
7. Write updated `mdvs.toml` (only `[fields]` section changes — all other config untouched)
8. If `auto_build` (or `--build`): trigger `build` (which internally runs check → embed)
9. Print result

**Disappearing fields:** a field in the toml that no longer appears in any file stays in the toml by default. Only during `--reinfer` or `--reinfer-all` is it removed (and the removal is reported).

**`--reinfer-all` vs `init --force`:** reinfer-all only re-runs field inference — all other config (`[scan]`, `[embedding_model]`, `[chunking]`, `[search]`) is preserved. `init --force` deletes and rewrites the entire `mdvs.toml`.

---

## Output

```rust
pub struct UpdateResult {
    pub files_scanned: usize,
    pub added: Vec<DiscoveredField>,     // new fields added to toml
    pub changed: Vec<ChangedField>,      // reinferred fields whose type changed
    pub removed: Vec<String>,            // fields removed (disappeared during reinfer)
    pub unchanged: usize,                // count of fields that stayed the same
    pub build: Option<BuildResult>,      // present if auto_build ran — see build.md
    pub dry_run: bool,
}

/// A field whose inferred type changed during reinfer.
pub struct ChangedField {
    pub name: String,
    pub old_type: String,
    pub new_type: String,
}
```

### Human format (new fields only)

```
Scanned 12 files

Added 2 fields:
  category  String   4/12
  author    String   7/12

Updated mdvs.toml
```

### Human format (reinfer)

```
Scanned 12 files

Changed 1 field:
  tags  String -> String[]

Removed 1 field:
  old_field  (no longer found)

2 fields unchanged

Updated mdvs.toml
```

### Human format (nothing to do)

```
Scanned 12 files — no changes
```

### Dry run

With `--dry-run`, nothing is written. Output ends with:

```
(dry run, nothing written)
```

---

## Errors

| Condition                              | Message                                            |
|----------------------------------------|----------------------------------------------------|
| `--reinfer` field not in toml          | `field '<name>' is not in mdvs.toml`               |
| `--reinfer` and `--reinfer-all` both   | `cannot use --reinfer and --reinfer-all together`  |
| Build failed (violations)              | reports violations, `build aborted — fix violations first` |

See also [Prerequisites](check.md#prerequisites) for toml validation errors.

---

## Examples

```bash
# Update: discover new fields
mdvs update

# Re-infer a specific field
mdvs update --reinfer tags

# Re-infer multiple fields
mdvs update --reinfer tags --reinfer author

# Re-infer all fields (keeps other config)
mdvs update --reinfer-all

# Preview changes without writing
mdvs update --dry-run

# Update without triggering build
mdvs update --build false

# Force build even if auto_build is false in toml
mdvs update --build true
```
