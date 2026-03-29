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

mdvs scans every markdown file, extracts frontmatter, and infers a typed schema. Each discovered field is shown as its own key-value table:

```
Initialized 43 files — 37 field(s)

┌ draft ───────────────────┬───────────────────────────────────────────────────┐
│ type                     │ Boolean                                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 8 out of 43                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ blog/**                                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ blog/**                                           │
└──────────────────────────┴───────────────────────────────────────────────────┘

...

┌ sensor_type ─────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 3 out of 43                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ projects/alpha/notes/**                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ projects/alpha/notes/**                           │
└──────────────────────────┴───────────────────────────────────────────────────┘

...

┌ title ───────────────────┬───────────────────────────────────────────────────┐
│ type                     │ String                                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ 37 out of 43                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ nullable                 │ false                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ required                 │ blog/**                                           │
│                          │ meetings/**                                       │
│                          │ people/**                                         │
│                          │ projects/**                                       │
│                          │ reference/protocols/**                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ allowed                  │ blog/**                                           │
│                          │ meetings/**                                       │
│                          │ people/**                                         │
│                          │ projects/**                                       │
│                          │ reference/protocols/**                            │
└──────────────────────────┴───────────────────────────────────────────────────┘

Initialized mdvs in 'example_kb'
```

That command did three things:

1. **Scanned** 43 markdown files and extracted their YAML frontmatter
2. **Inferred** 37 typed fields — strings, integers, floats, booleans, arrays, even a nested object (`calibration`)
3. **Wrote** `mdvs.toml` with the inferred schema

Notice the `files` row: `draft` appears in 8 out of 43 files — all in `blog/`. `sensor_type` in 3 out of 43 — all in `projects/alpha/notes/`. mdvs captured not just the types, but *where* each field belongs, via the `required` and `allowed` glob patterns.

Here's what a field definition looks like in `mdvs.toml`:

```toml
[[fields.field]]
name = "sensor_type"
type = "String"
allowed = ["projects/alpha/notes/**"]
required = ["projects/alpha/notes/**"]
nullable = false
```

This means `sensor_type` is allowed only in experiment notes, and required there. If it appears in a blog post, `check` will flag it. If it's missing from an experiment note, `check` will flag that too.

One artifact is created by `init`: **`mdvs.toml`** — the schema file. Commit this to version control. The `.mdvs/` directory (search index) is created later on first `build` or `search`.

## Validate

Check that every file conforms to the schema:

```bash
mdvs check example_kb
```

```
Checked 43 files — no violations
```

Since `mdvs init` just inferred the schema from these same files, everything passes. The power of `check` comes after you tighten the schema — or when files drift from it. Try adding `sensor_type: SPR-A1` to a blog post — mdvs will flag it as `Disallowed` because that field doesn't belong there.

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

Violations (4):
┌ convergence_ms ──────────┬───────────────────────────────────────────────────┐
│ kind                     │ Wrong type                                        │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ type Boolean                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/beta/notes/initial-findings.md (got Inte │
│                          │ ger)                                              │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ drift_rate ──────────────┬───────────────────────────────────────────────────┐
│ kind                     │ Null value not allowed                            │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ not nullable                                      │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/alpha/notes/experiment-2.md              │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ firmware_version ────────┬───────────────────────────────────────────────────┐
│ kind                     │ Not allowed                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ allowed in ["people/interns/**"]                  │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ people/remo.md                                    │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ observation_notes ───────┬───────────────────────────────────────────────────┐
│ kind                     │ Missing required                                  │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ rule                     │ required in ["projects/alpha/notes/**"]           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ files                    │ projects/alpha/notes/experiment-1.md              │
│                          │ projects/alpha/notes/experiment-2.md              │
└──────────────────────────┴───────────────────────────────────────────────────┘
```

Four violation types, each catching a different kind of problem:

| Violation | Meaning |
|---|---|
| `Missing required` | A file in a required path is missing the field |
| `Wrong type` | The value doesn't match the declared type |
| `Null value not allowed` | The field is present but `null`, and `nullable` is `false` |
| `Not allowed` | The field appears in a file outside its `allowed` paths |

Each violation table shows the field name, the kind of violation, the violated rule, and the affected files. See [check](./commands/check.md) for the full reference.

Revert your changes to `mdvs.toml` before continuing (or re-run `mdvs init example_kb --force` to regenerate it).

## Search

Query the index with natural language. On first run, `search` auto-builds the index:

> **Note:** The first `search` or `build` downloads the embedding model from HuggingFace (~30 MB for the default model). This is a one-time download — subsequent runs use the cached model and start instantly.

```bash
mdvs search "calibration" example_kb
```

```
Searched "calibration" — 10 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ calibration                                       │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/meetings/2031-06-15.md             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.585                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ lines                    │ 14-22                                             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ text                     │ # Alpha Kickoff — Calibration Campaign ...        │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #2 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/alpha/meetings/2031-10-10.md             │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.501                                             │
...

...
```

Results are ranked by semantic similarity — not keyword matching. The `score` is cosine similarity (higher means more similar). The `text` row shows the best-matching chunk from each file.

### Filtering with `--where`

Add a SQL filter on any frontmatter field:

```bash
mdvs search "quantum" example_kb --where "status = 'active'"
```

```
Searched "quantum" — 3 hits

┌──────────────────────────┬───────────────────────────────────────────────────┐
│ query                    │ quantum                                           │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ model                    │ minishlab/potion-base-8M                          │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ limit                    │ 10                                                │
└──────────────────────────┴───────────────────────────────────────────────────┘

┌ #1 ──────────────────────┬───────────────────────────────────────────────────┐
│ file                     │ projects/beta/overview.md                         │
├──────────────────────────┼───────────────────────────────────────────────────┤
│ score                    │ 0.123                                             │
...

...
```

Only files with `status: active` in their frontmatter are included. The `--where` clause supports any SQL expression — boolean logic, comparisons, array functions, and more. See the [Search Guide](./search-guide.md) for the full syntax.

## What's next

- **[Concepts](./concepts.md)** — How schema inference, types, and validation work under the hood
- **[Commands](./commands/init.md)** — Full reference for every command and flag
- **[Configuration](./configuration.md)** — Customize `mdvs.toml` to tighten your schema
- **[Search Guide](./search-guide.md)** — Complex queries: arrays, nested objects, combined filters
