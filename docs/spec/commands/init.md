# `mdvs init`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md)

---

## Synopsis

```
mdvs init [path] [flags]
```

| Flag                  | Type       | Default                    | Description                              |
|-----------------------|------------|----------------------------|------------------------------------------|
| `path`                | positional | `.`                        | Directory to scan                        |
| `--glob`              | string     | `**`                       | File glob pattern                        |
| `--force`             | bool       | false                      | Overwrite existing `mdvs.toml`           |
| `--dry-run`           | bool       | false                      | Print discovery table, write nothing     |
| `--ignore-bare-files` | bool       | false                      | Exclude files without frontmatter        |
| `--auto-build`        | bool       | true                       | Run build after init, write build config |
| `--model`             | string     | `minishlab/potion-base-8M` | HuggingFace model ID (requires `--auto-build`) |
| `--revision`          | string     | (latest)                   | Pin model to specific commit SHA (requires `--auto-build`) |
| `--chunk-size`        | usize      | 1024                       | Max chunk size in characters (requires `--auto-build`) |

---

## Behavior

1. Validate path is a directory
2. Check `mdvs.toml` doesn't exist (unless `--force`)
3. Validate flag combinations (see Errors)
4. Scan markdown files matching glob
5. Infer schema (fields, types, allowed/required globs)
6. Collect `InitResult`
7. If `--dry-run`: print result, return
8. Write `mdvs.toml`:
   - Always: `[scan]`, `[update]`, `[fields]` + `[[fields.field]]`
   - Only if `--auto-build`: `[embedding_model]`, `[chunking]`, `[search]`
9. If `--auto-build`: run `build` (download model, create `.mdvs/`, write parquets)
10. Print result

Progress messages ("Loading model...", "Building index...") go to **stderr**.
The formatted result goes to **stdout**.

---

## Output

```rust
pub struct InitResult {
    pub path: PathBuf,
    pub files_scanned: usize,
    pub fields: Vec<DiscoveredField>,  // see shared.md
    pub build: Option<BuildResult>,    // see build.md — present only when auto_build
    pub dry_run: bool,
}
```

### Human format (no build)

```
3 files scanned

 Field  Type     Count
 ─────────────────────
 title  String   3/3
 tags   Array    2/3
 draft  Boolean  1/3

Initialized mdvs in '/path/to/vault'
```

### Human format (with build)

```
3 files scanned

 Field  Type     Count
 ─────────────────────
 title  String   3/3
 tags   Array    2/3
 draft  Boolean  1/3

Built index: <build summary line TBD>

Initialized mdvs in '/path/to/vault'
```

### Dry run

With `--dry-run`, nothing is written. The last line becomes:

```
(dry run, nothing written)
```

If `--auto-build --dry-run`, the output also includes:

```
Would build index with model 'minishlab/potion-base-8M'
(dry run, nothing written)
```

---

## Errors

| Condition                              | Message                                                              |
|----------------------------------------|----------------------------------------------------------------------|
| Path is not a directory                | `'<path>' is not a directory`                                        |
| `mdvs.toml` exists, no `--force`      | `mdvs.toml already exists in '<path>' (use --force to overwrite)`    |
| No markdown files found                | `no markdown files found in '<path>'`                                |
| `--model` without `--auto-build`       | `--model has no effect without --auto-build`                         |
| `--revision` without `--auto-build`    | `--revision has no effect without --auto-build`                      |
| `--chunk-size` without `--auto-build`  | `--chunk-size has no effect without --auto-build`                    |

---

## Examples

```bash
# Initialize current directory with defaults (includes build)
mdvs init

# Initialize a specific directory
mdvs init ~/notes

# Dry run to see what would be discovered
mdvs init --dry-run

# Use a different model, pin revision
mdvs init --model minishlab/potion-base-32M --revision a1b2c3d

# Overwrite existing config
mdvs init --force

# Only index files under blog/
mdvs init --glob "blog/**"

# Validation only — no model download, no build
mdvs init --auto-build false

# Dry run with build preview
mdvs init --dry-run --auto-build true
```
