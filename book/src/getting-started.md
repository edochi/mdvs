# Getting Started

Install mdvs, run it on a real directory, and search your first query — all in under five minutes.

## Install

```bash
cargo install mdvs
```

You need a working [Rust toolchain](https://rustup.rs/). Prebuilt binaries will be available once the crate is published.

## Get the example files

This book uses a fixture called `example_kb` — a fictional research lab's knowledge base with ~46 markdown files, varied frontmatter, and a few deliberate inconsistencies. Clone the repo to follow along:

```bash
git clone https://github.com/edochi/mdvs.git
cd mdvs
```

## Initialize

Run `mdvs init` on the example directory:

```bash
mdvs init example_kb
```

mdvs scans every markdown file, extracts frontmatter, infers a typed schema, and builds a search index:

```
Initialized 43 files — 37 field(s)

╭─────────────────────┬───────────────────────┬───────┬────────────────────────╮
│ "action_items"      │ String[]              │ 9/43  │                        │
│ "algorithm"         │ String                │ 2/43  │                        │
│ "ambient_humidity"  │ Float                 │ 1/43  │                        │
│ "approved_by"       │ String                │ 4/43  │                        │
│ "attendees"         │ String[]              │ 10/43 │                        │
│ "author"            │ String                │ 18/43 │                        │
│ "author's_note"     │ String                │ 3/43  │ ' → '' in --where      │
│ "calibration"       │ {adjusted: {intensity │ 2/43  │                        │
│                     │ : Float, wavelength:  │       │                        │
│                     │ Float}, baseline: {in │       │                        │
│                     │ tensity: Float, notes │       │                        │
│                     │ : String, wavelength: │       │                        │
│                     │  Float}}              │       │                        │
│ "commission_date"   │ String                │ 1/43  │                        │
│ "convergence_ms"    │ Integer               │ 1/43  │                        │
│ "dataset"           │ String                │ 2/43  │                        │
│ "date"              │ String                │ 17/43 │                        │
│ "draft"             │ Boolean               │ 8/43  │                        │
│ "drift_rate"        │ Float?                │ 3/43  │                        │
│ "duration_minutes"  │ Integer               │ 10/43 │                        │
│ "email"             │ String                │ 4/43  │                        │
│ "equipment_id"      │ String                │ 2/43  │                        │
│ "firmware_version"  │ String                │ 1/43  │                        │
│ "joined"            │ String                │ 5/43  │                        │
│ "lab section"       │ String                │ 4/43  │ use "field name" in -- │
│                     │                       │       │ where                  │
│ "last_reviewed"     │ String                │ 4/43  │                        │
│ "notes"v2""         │ Boolean               │ 1/43  │ " → "" in --where      │
│ "observation_notes" │ String                │ 1/43  │                        │
│ "priority"          │ String                │ 7/43  │                        │
│ "project"           │ String                │ 4/43  │                        │
│ "publications"      │ Integer               │ 2/43  │                        │
│ "review_score"      │ String?               │ 1/43  │                        │
│ "role"              │ String                │ 5/43  │                        │
│ "sample_count"      │ Integer               │ 3/43  │                        │
│ "sensor_type"       │ String                │ 3/43  │                        │
│ "specialization"    │ String                │ 2/43  │                        │
│ "status"            │ String                │ 17/43 │                        │
│ "tags"              │ String[]              │ 16/43 │                        │
│ "title"             │ String                │ 37/43 │                        │
│ "unit_id"           │ String                │ 1/43  │                        │
│ "version"           │ String                │ 4/43  │                        │
│ "wavelength_nm"     │ Float                 │ 3/43  │                        │
╰─────────────────────┴───────────────────────┴───────┴────────────────────────╯

Initialized mdvs in 'example_kb'

Built index — 43 files, 59 chunks (full rebuild)

╭─────────────────────────┬─────────────────────────┬──────────────────────────╮
│ embedded                │ 43 files                │ 59 chunks                │
╰─────────────────────────┴─────────────────────────┴──────────────────────────╯
```

That one command did several things:

1. **Scanned** 43 markdown files and extracted their YAML frontmatter
2. **Inferred** 37 typed fields — strings, integers, floats, booleans, arrays, even a nested object (`calibration`)
3. **Wrote** `mdvs.toml` with the inferred schema
4. **Embedded** the file contents into vectors and stored them in `.mdvs/`

Two artifacts were created:

- **`mdvs.toml`** — the schema file. Commit this to version control.
- **`.mdvs/`** — the search index (Parquet files). Add this to `.gitignore`.

## Validate

Check that every file conforms to the schema:

```bash
mdvs check example_kb
```

```
Checked 43 files — no violations
```

Since `mdvs init` just inferred the schema from these same files, everything passes. The power of `check` comes after you tighten the schema.

### What violations look like

Open `mdvs.toml` and make a few changes to tighten the constraints:

- Require `observation_notes` in all experiment files (currently optional)
- Change `convergence_ms` type from `Integer` to `Boolean` (simulating a type mismatch)
- Set `drift_rate` to non-nullable (one file has `drift_rate: null`)
- Restrict `firmware_version` to only appear in `people/interns/**` (it currently appears in `people/*`)

Run `check` again:

```bash
mdvs check example_kb
```

```
Checked 43 files — 4 violation(s)

╭───────────────────────────────┬───────────────────────────┬──────────────────╮
│ "convergence_ms"              │ WrongType                 │ 1 file           │
│ "drift_rate"                  │ NullNotAllowed            │ 1 file           │
│ "firmware_version"            │ Disallowed                │ 1 file           │
│ "observation_notes"           │ MissingRequired           │ 2 files          │
╰───────────────────────────────┴───────────────────────────┴──────────────────╯
```

Four violation types, each catching a different kind of problem:

| Violation | Meaning |
|---|---|
| **MissingRequired** | A file in a required path is missing the field |
| **WrongType** | The value doesn't match the declared type |
| **NullNotAllowed** | The field is present but `null`, and `nullable` is `false` |
| **Disallowed** | The field appears in a file outside its `allowed` paths |

This is the compact output — it groups violations by field. Add `-v` for verbose output showing every affected file and the specific value that caused the violation. See [check](./commands/check.md) for the full reference.

Revert your changes to `mdvs.toml` before continuing (or re-run `mdvs init example_kb --force` to regenerate it).

## Search

Query the index with natural language:

```bash
mdvs search "calibration" example_kb
```

```
Searched "calibration" — 10 hits

╭────────────┬──────────────────────────────────────────────────┬──────────────╮
│ 1          │ "projects/alpha/meetings/2031-06-15.md"          │ 0.585        │
│ 2          │ "projects/alpha/meetings/2031-10-10.md"          │ 0.501        │
│ 3          │ "projects/alpha/notes/experiment-1.md"           │ 0.478        │
│ 4          │ "blog/drafts/upcoming-talk.md"                   │ 0.470        │
│ 5          │ "blog/published/2032/q1/new-equipment.md"        │ 0.466        │
│ 6          │ "meetings/all-hands/2032-01.md"                  │ 0.465        │
│ 7          │ "projects/alpha/overview.md"                     │ 0.462        │
│ 8          │ "projects/beta/overview.md"                      │ 0.449        │
│ 9          │ "reference/tools.md"                             │ 0.445        │
│ 10         │ "people/remo.md"                                 │ 0.437        │
╰────────────┴──────────────────────────────────────────────────┴──────────────╯
```

Results are ranked by semantic similarity — not keyword matching. The score column is cosine distance (lower means more similar).

### Filtering with `--where`

Add a SQL filter on any frontmatter field:

```bash
mdvs search "quantum" example_kb --where "status = 'active'"
```

```
Searched "quantum" — 3 hits

╭───────────────┬──────────────────────────────────────────┬───────────────────╮
│ 1             │ "projects/beta/overview.md"              │ 0.123             │
│ 2             │ "projects/alpha/overview.md"             │ 0.101             │
│ 3             │ "projects/alpha/budget.md"               │ 0.055             │
╰───────────────┴──────────────────────────────────────────┴───────────────────╯
```

Only files with `status: active` in their frontmatter are included. The `--where` clause supports any SQL expression — boolean logic, comparisons, array functions, and more. See the [Search Guide](./search-guide.md) for the full syntax.

## What's next

- **[Concepts](./concepts.md)** — How schema inference, types, and validation work under the hood
- **[Commands](./commands/init.md)** — Full reference for every command and flag
- **[Configuration](./configuration.md)** — Customize `mdvs.toml` to tighten your schema
- **[Search Guide](./search-guide.md)** — Complex queries: arrays, nested objects, combined filters
