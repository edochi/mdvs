# `mdvs init`

**Status: DRAFT**

**See also:** [Shared Types](../shared.md), [Workflow: Inference](../workflows/inference.md)

---

## Synopsis

```
mdvs init [path] [flags]
```

| Flag                  | Type       | Default                    | Description                          |
|-----------------------|------------|----------------------------|--------------------------------------|
| `path`                | positional | `.`                        | Directory to scan                    |
| `--model`             | string     | `minishlab/potion-base-8M` | HuggingFace model ID                 |
| `--revision`          | string     | (latest)                   | Pin model to specific commit SHA     |
| `--glob`              | string     | `**`                       | File glob pattern                    |
| `--force`             | bool       | false                      | Overwrite existing config/lock       |
| `--dry-run`           | bool       | false                      | Print discovery table, write nothing |
| `--ignore-bare-files` | bool       | false                      | Exclude files without frontmatter    |
| `--chunk-size`        | usize      | 1024                       | Max chunk size in characters         |
| `--auto-build`        | bool       | true                       | Write auto_build preference to toml  |

---

## Behavior

1. Validate path is a directory
2. Check `mdvs.toml` doesn't exist (unless `--force`)
3. Scan markdown files matching glob
4. Infer schema (fields, types, allowed/required globs)
5. Collect `InitResult`
6. If `--dry-run`: format and print result with "(dry run)" note, return
7. Download model, resolve revision
8. Write `mdvs.toml`
9. Write `mdvs.lock`
10. Create `.mdvs/` directory
11. If `auto_build`: run `build` (full pipeline)
12. Format and print result to stdout

Progress messages ("Loading model...", "Building index...") go to **stderr**.
The formatted result goes to **stdout**.

---

## Output

```rust
pub struct InitResult {
    pub files_scanned: usize,
    pub fields: Vec<DiscoveredField>,  // see shared.md
    pub model: String,
    pub model_revision: Option<String>,
    pub dry_run: bool,
}
```

### Human format

```
3 files scanned

 Field  Type     Count
 ─────────────────────
 title  String   3/3
 tags   Array    2/3
 draft  Boolean  1/3

Initialized mdvs in '/path/to/vault'
```

With `--dry-run`, the last line becomes:

```
(dry run, nothing written)
```

---

## Errors

| Condition                     | Message                                                              |
|-------------------------------|----------------------------------------------------------------------|
| Path is not a directory       | `'<path>' is not a directory`                                        |
| mdvs.toml exists, no --force | `mdvs.toml already exists in '<path>' (use --force to overwrite)`    |
| No markdown files found       | `no markdown files found in '<path>'`                                |

---

## Examples

```bash
# Initialize current directory with defaults
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

# Skip auto-build after init
mdvs init --auto-build false
```
