# Schema Inference

mdvs infers a typed schema from your files automatically — no manual schema definition needed. Run `mdvs init`, and it scans every markdown file, extracts frontmatter, infers types, and computes path patterns that describe where each field appears. The result is `mdvs.toml`, which you can then tighten by hand.

## What gets scanned

mdvs walks your directory and includes every `.md` and `.markdown` file that matches the `glob` pattern in `[scan]`:

```toml
[scan]
glob = "**"
include_bare_files = true
skip_gitignore = false
```

Three settings control what's included:

| Setting | Default | Effect |
|---|---|---|
| `glob` | `"**"` | Which files to scan. Use narrower globs to exclude subtrees. |
| `include_bare_files` | `true` | Whether to include files without any YAML frontmatter |
| `skip_gitignore` | `false` | Whether to ignore `.gitignore` patterns during scan |

mdvs also respects `.mdvsignore` files (same syntax as `.gitignore`) for excluding paths from scanning without touching your `.gitignore`.

### Bare files vs empty frontmatter

These look similar but are different:

**Bare file** — no frontmatter fences at all:

```markdown
This file has no frontmatter. Just content.
```

**Empty frontmatter** — fences with nothing between them:

```markdown
---
---
This file has frontmatter, but zero fields.
```

In `example_kb`, four files are bare (`scratch.md`, `lab-values.md`, `reference/tools.md`, `reference/glossary.md`) and one has empty frontmatter (`reference/quick-start.md`).

Both types contribute zero fields to inference. The difference matters for validation: a bare file is excluded entirely when `include_bare_files = false`, while an empty-frontmatter file is always included (it has frontmatter — just none with fields).

## From files to fields

For each scanned file, mdvs extracts the YAML frontmatter and infers a type for every key. When the same field appears across multiple files, its type is widened to a common type (see [Types & Widening](./types.md) for the full rules).

In `example_kb`, scanning 43 files produces 37 distinct field names. Some fields like `title` appear in 37 files. Others like `unit_id` appear in just one.

The output of this step is a list of fields, each with:
- A **name**
- A **type** (widened across all files where it appears)
- A **nullable** flag (true if any file had a `null` value)
- The **set of files** where it was found

## Path patterns

The most interesting part of inference is how mdvs computes **where** each field belongs. It produces two sets of glob patterns per field:

- **`allowed`** — where the field *may* appear. Any file matching these patterns can have the field without triggering a violation.
- **`required`** — where the field *must* appear. Any file matching these patterns that's missing the field triggers a `MissingRequired` violation.

### How patterns are computed

mdvs builds a directory tree from the scanned files and works bottom-up:

1. For each directory, it tracks which fields appear in **all** files (intersection) and which appear in **any** file (union)
2. When a field appears in every file under a directory and its subdirectories, it collapses into a recursive glob (`dir/**`)
3. When a field appears in some but not all files, only `allowed` gets the glob — `required` does not

The result is a minimal set of globs that describes the field's distribution.

### Examples from `example_kb`

**Narrow and consistent** — `sensor_type` appears in all three experiment notes and nowhere else:

```toml
[[fields.field]]
name = "sensor_type"
type = "String"
allowed = ["projects/alpha/notes/**"]
required = ["projects/alpha/notes/**"]
```

`allowed` and `required` are the same — every file that has this field is in the same directory, and every file in that directory has it.

**Broad and consistent** — `title` appears in 37 of 43 files across many directories:

```toml
[[fields.field]]
name = "title"
type = "String"
allowed = ["blog/**", "meetings/**", "people/**", "projects/**", "reference/protocols/**"]
required = ["blog/**", "meetings/**", "people/**", "projects/**", "reference/protocols/**"]
```

Again, `allowed` equals `required` — every file in those directories has a `title`. The five directories without `title` are bare files at the root and in `reference/`.

**Allowed broader than required** — `email` exists in all `people/` files except one:

```toml
[[fields.field]]
name = "email"
type = "String"
allowed = ["people/**"]
required = ["people/interns/**"]
```

`allowed` is `people/**` — the field may appear anywhere under `people/`. But `required` is only `people/interns/**` — the one subdirectory where every file happens to have it. In `people/*` (the non-intern profiles), some have `email` and some don't, so it can't be required there.

**Present but never required** — `ambient_humidity` appears in only one of three experiment notes:

```toml
[[fields.field]]
name = "ambient_humidity"
type = "Float"
allowed = ["projects/alpha/notes/**"]
required = []
```

`required` is empty — the field never appears in *every* file under any directory, so mdvs can't require it anywhere.

### The pattern

The general rule is `required ⊆ allowed` — you can't require a field somewhere it's not allowed. Within that:

- `required = allowed` when every file in a directory has the field
- `required ⊂ allowed` when the field is consistent in some directories but sporadic in others
- `required = []` when the field is sporadic — present in some files but not consistently in any directory

## The three field states

Every field in `mdvs.toml` is in one of three states:

### Constrained

Listed under `[[fields.field]]`. Validation enforces type, allowed paths, required paths, and nullable. `mdvs update` preserves constrained fields unless you explicitly use `update reinfer`.

```toml
[[fields.field]]
name = "draft"
type = "Boolean"
allowed = ["blog/**"]
required = ["blog/**"]
nullable = false
```

Only `name` is required — properties you omit use permissive defaults:

| Property | Default | Meaning |
|---|---|---|
| `type` | `String` | Accepts any value (String is the top type) |
| `allowed` | `["**"]` | Allowed in every file |
| `required` | `[]` | Not required anywhere |
| `nullable` | `true` | Null values accepted |

A `[[fields.field]]` with just a name is effectively unconstrained, but still known — useful when you want to acknowledge a field without committing to specific constraints yet.

### Ignored

Listed in the `ignore` array. The field is known but not validated — no type checks, no path checks. `mdvs update` skips ignored fields entirely.

```toml
[fields]
ignore = ["internal_notes", "scratch_data"]
```

Use this for fields you don't want to enforce — temporary fields, fields in flux, or fields you've decided aren't worth constraining.

### Unknown

Not mentioned in `mdvs.toml` at all. When `mdvs update` finds a field that isn't constrained or ignored, it reports it as a **new field** and adds it to the schema.

A field can be in exactly one state. Moving a field from constrained to ignored means removing its `[[fields.field]]` entry and adding its name to `ignore`. Moving it back means the reverse.

## Keeping the schema current

After initial inference with `mdvs init`, the schema is a snapshot of your files at that moment. As files change — new fields appear, old ones shift — use `mdvs update` to bring the schema up to date.

### Default mode

```bash
mdvs update example_kb
```

Only **new** fields are added. Existing fields are left untouched, even if their types or paths have changed. This is conservative by design — your manual edits to `mdvs.toml` are preserved.

Fields that disappear from all files still stay in the toml. This prevents accidental removal when files are temporarily missing.

### Re-inferring specific fields

```bash
mdvs update example_kb reinfer tags
```

Treats `tags` as if it had never been seen — removes it from the schema, re-scans, and infers it fresh. Use this when you've fixed bad data (like a `tags: [1, 2, 3]` that should have been strings) and want the type or paths to update.

### Re-inferring everything

```bash
mdvs update example_kb reinfer
```

When no fields are named, every field is reinferred. The entire `[[fields.field]]` section is rebuilt from scratch, but all other config (`[scan]`, `[embedding_model]`, etc.) is preserved.

This is different from `mdvs init --force`, which overwrites the entire `mdvs.toml` including non-field config.

## Edge cases

- **Fields in a single file** — get a narrow `allowed` glob matching just that file's directory. Example: `unit_id` only in `people/remo.md` → `allowed = ["people/*"]`.
- **Null-only fields** — type defaults to String (see [Types & Widening](./types.md#nullable)). Example: `review_score` is always `null` → `String?`.
- **Special characters in field names** — names with spaces (`lab section`), single quotes (`author's_note`), or double quotes (`notes"v2"`) are preserved as-is. They need quoting in `--where` clauses (see [Search Guide](../search-guide.md)).
- **Empty arrays** `[]` — element type defaults to String, giving `String[]`. If real values appear later, use `update reinfer` to pick up the correct element type.
