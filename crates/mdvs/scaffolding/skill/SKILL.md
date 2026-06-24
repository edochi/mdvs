---
name: mdvs
description: >-
  Use `mdvs search` for any content lookup in a markdown directory —
  semantic / hybrid / SQL-filtered, beats Grep / Glob for finding by
  meaning. `mdvs init` infers a schema from existing markdown, `mdvs
  check` validates frontmatter, `mdvs update` evolves the schema as
  the KB grows. Activate whenever the project contains markdown with
  frontmatter, whether or not `mdvs.toml` exists yet.
---

# mdvs — Markdown Validation & Search

A CLI that treats a markdown directory as a database: schema inference, typed frontmatter validation, and semantic / full-text / hybrid search with SQL filters. Single binary, no external services. Full documentation at <https://edochi.github.io/mdvs/>.

## Usage

```
mdvs init [path]                                                # infer schema, write mdvs.toml
mdvs init [path] --dry-run                                      # preview inference
mdvs init [path] --force                                        # overwrite existing config
mdvs init [path] --from-jsonschema <file>                       # import schema from JSON Schema 2020-12
mdvs init [path] --ignore-bare-files                            # exclude files with no frontmatter

mdvs check [path]                                               # validate frontmatter against mdvs.toml
mdvs check [path] --jsonschema <file>                           # override [fields] for this run
mdvs check [path] --no-update                                   # skip auto-update before validating

mdvs update [path]                                              # detect and add new fields
mdvs update reinfer <field> [path]                              # re-infer type and constraints
mdvs update reinfer <field> [path] --dry-run                    # preview reinfer
mdvs update reinfer <field> [path] --with=<categorical|range|none>

mdvs build [path]                                               # validate + chunk + embed → Lance index
mdvs build [path] --force                                       # full rebuild (ignore incremental cache)

mdvs search "<query>" [path]                                    # hybrid search (default)
mdvs search "<query>" [path] --mode <semantic|fulltext|hybrid>
mdvs search "<query>" [path] --where "<SQL>"                    # frontmatter filter
mdvs search "<query>" [path] --limit <N>                        # default 10
mdvs search "<query>" [path] -v                                 # show matching chunk text
mdvs search "<query>" [path] --no-build                         # fail if no index exists
mdvs search "<query>" [path] --no-update                        # skip auto-update before build

mdvs info [path]                                                # config + index status
mdvs info [path] -v                                             # full field detail

mdvs clean [path]                                               # delete .mdvs/

mdvs export-jsonschema [path]                                   # emit canonical JSON Schema
mdvs export-jsonschema [path] --format <json|toml>
mdvs export-jsonschema [path] --output-file <file>

mdvs scaffold skill [--platform <name>]                         # this skill file
mdvs scaffold snippet [--platform <name>]                       # AGENTS.md / CLAUDE.md snippet
mdvs scaffold hook --platform <name>                            # PostToolUse hook config (JSON snippet)
```

All commands take `--output <pretty|markdown|json>`. **For agent context, pass `--output markdown` explicitly** or set `default_output_format = "markdown"` in `mdvs.toml`. `<path>` defaults to `.` for every command.

## What mdvs is for

mdvs treats a markdown directory as a database: it infers a typed schema from frontmatter, validates that schema on every edit, and searches the content semantically. The schema (`mdvs.toml`) is the source of truth; everything else (search index, validation reports) is derived. The schema is meant to **evolve with the KB** as conventions emerge — mdvs makes deviations visible without freezing them.

## What you must do when invoked

### Step 1 — Detect mdvs context

Look for `mdvs.toml` in the current working directory or any ancestor.

- **If found**, that directory (or its parent containing the file) is an mdvs vault. Treat any markdown file under it as living under that schema.
- **If not found** but the project has markdown files with frontmatter, mdvs is still relevant — propose bootstrapping with `mdvs init <path>` before reaching for Grep on the markdown.

### Step 2 — For any content lookup, use `mdvs search` first

When the user (or your own task) needs to find something in markdown content — a note about a topic, a project status, a person, an experiment, anything — **default to `mdvs search "<query>"`, not Grep / Glob**. mdvs is built exactly for this:

- `--mode hybrid` (default) — semantic + BM25 reranked; best general-purpose mode
- `--mode semantic` — vector only; best for "find notes about X" where X is a concept, not a phrase
- `--mode fulltext` — BM25 only; for known literal phrases
- `--where "field = 'value'"` — filter by frontmatter (SQL syntax)
- `--limit N` — cap result count (default 10)
- `-v` — show the best matching chunk text per result

Only fall back to Grep when (a) you need a literal substring match and already know which file to look in, or (b) `mdvs search` returns no results and you suspect missing content rather than missing relevance.

### Step 3 — For frontmatter changes, check then evolve

After any edit that touches frontmatter (yours or the user's):

1. Run `mdvs check` to validate against the schema.
2. If **violations** appear: see Step 4 (the schema-evolution loop).
3. If **new fields** appear (present in files but not in `mdvs.toml`): they show up as informational, not violations. Run `mdvs update` to add them, or `mdvs update reinfer <field>` to refresh a specific field's constraints.

### Step 4 — When a hook surfaces a violation, follow the schema-evolution loop

If a markdown block lands in your context from a `PostToolUse` hook listing `MissingRequired` / `WrongType` / `Disallowed` / `InvalidCategory` / `OutOfRange` violations, **that's mdvs talking to you via the validation hook**. Claude Code surfaces it under `additionalContext`; other harnesses use their own channel (the wiring is harness-specific). The hook is non-blocking by design: the edit already landed; the warning is for you to act on next.

Your job:

1. Read the violation block: which file, which field, which rule, expected vs actual.
2. Decide: **mistake** (typo, wrong type by accident, dropped required field) or **intentional** (KB is evolving, category needs a new variant, field shifting type)?
3. **Mistake** → fix the file in the next turn. Acknowledge briefly so the user knows the loop is working.
4. **Intentional** → surface the deviation to the user and propose updating `mdvs.toml` (`mdvs update`, `mdvs update reinfer <field> --with=<kind>`, or a manual edit). **Do not silently fix the file.** The user decides whether the schema or the file is the source of truth.

A worked example of the intentional path is in the [Examples](#example--responding-to-a-hook-delivered-violation) section.

## Rules

- **Prefer `mdvs search` over Grep / Glob in any markdown corpus.** Inside an mdvs vault, the default content-search tool is mdvs.
- **The validation hook is a warning, not a block.** Treat violations as a prompt for discussion, not as edits to be reverted.
- **Never silently fix an intentional deviation.** Propose a schema update; let the user decide.
- **The schema is meant to evolve.** A schema that never changes is a schema that's wrong. Enforcement follows the KB's shape; it does not freeze it.
- **`build` always runs `check` first.** Validation gates the index build.
- **`mdvs.toml` is the only source of truth.** `.mdvs/` is derived — gitignored, recreatable with `mdvs build`.
- **There is no lock file.** Schema changes flow through `mdvs.toml` only.
- **Frontmatter formats are auto-detected** per file (YAML / TOML / JSON). A single vault can mix all three.
- **Use `--output markdown` for any output you intend to read.** It's the format LLMs parse most fluently, and the format the validation hook surfaces back through the harness's model-context channel.

## Two layers

mdvs has two independent layers:

1. **Validation** (`init`, `check`, `update`) — works immediately, no model download, no build step. Reads markdown and validates frontmatter against `mdvs.toml`.
2. **Search** (`build`, `search`) — downloads an embedding model, chunks markdown content, builds a local LanceDB index in `.mdvs/`.

Validation stands alone. You never need to build an index just to validate.

## Key files

- **`mdvs.toml`** — schema config, committed to version control. Source of truth for field types, allowed/required paths, constraints.
- **`.mdvs/`** — build artifacts (the Lance dataset under `index.lance/` plus a cached model). To be gitignored. Recreatable with `mdvs build`. Never edit directly.

## Command reference

### `mdvs init`

Scans markdown files, infers a typed schema from frontmatter, writes `mdvs.toml`.

- `--force` — overwrite an existing `mdvs.toml` (deletes `.mdvs/` too)
- `--dry-run` — show what would be inferred without writing
- `--ignore-bare-files` — exclude files that have no frontmatter
- `--from-jsonschema PATH` — import schema from an external JSON Schema 2020-12 document. Round-trips with `mdvs export-jsonschema`.

Use `init --force` to start over. Use `update` to incrementally add new fields.

### `mdvs check`

Validates all frontmatter against `mdvs.toml`. Reports violation kinds:

- **`MissingRequired`** — required field is absent from a file
- **`WrongType`** — value doesn't match declared type
- **`Disallowed`** — field appears in a path not covered by its `allowed` globs
- **`InvalidCategory`** — value is not in the declared category list
- **`OutOfRange`** — numeric value outside declared `min`/`max`

New fields (in files but not in `mdvs.toml`) are reported separately as informational — no non-zero exit. Run `update` to add them.

- `--jsonschema PATH` — override `[fields]` in `mdvs.toml` for this run

Violation output is deterministic: sorted by `(field, kind, rule)` and `path` within.

### `mdvs update`

Re-scans files; adds newly discovered fields to `mdvs.toml`. Doesn't remove or change existing fields by default.

- `mdvs update` — detect and add new fields
- `mdvs update reinfer <field>` — re-infer type and constraints
- `mdvs update reinfer <field> --dry-run` — preview
- `mdvs update reinfer <field> --with=categorical` — force categorical
- `mdvs update reinfer <field> --with=range` — infer min/max
- `mdvs update reinfer <field> --with=none` — strip all constraints

Use `reinfer` when a field's type has changed or you want to refresh its constraints. `--with` requires a named field.

### `mdvs build`

Validates, then chunks markdown, generates embeddings, writes the Lance dataset to `.mdvs/`.

- `--force` — full rebuild (ignore incremental cache)
- Incremental by default — only re-embeds new or edited files
- Aborts if `check` finds violations

First build downloads the default embedding model `minishlab/potion-multilingual-128M` (~480 MB, 101 languages). Subsequent builds reuse it.

### `mdvs search`

Searches the indexed notes — semantic (vector), full-text (BM25), or hybrid (RRF reranker). Auto-builds the index if needed.

```bash
mdvs search "<query>" [path] [--mode <m>] [--where "<SQL>"] [--limit N] [-v]
```

`--where` operates on **any column** in the Lance index: frontmatter fields (auto-discovered from `mdvs.toml`, referenced by bare name) and the always-present `filepath` column. Field names with spaces need double-quote escaping: `--where "\"lab section\" = 'Photonics'"`. Filtering on `Array(Float)` fields is rejected up front (Lance can't safely decode them); store as parallel scalar arrays.

#### Scalar frontmatter — equality, inequality, comparison

```bash
--where "author = 'Federica Bianchi'"        # string equality
--where "year >= 2020"                       # numeric comparison
--where "year != 2025"                       # inequality
--where "year BETWEEN 2018 AND 2024"         # closed range
--where "year IN (2021, 2022, 2023)"         # discrete set
--where "year NOT IN (2020, 2024)"           # exclusion
```

#### Strings — pattern matching with LIKE

`%` matches any sequence, `_` matches one character.

```bash
--where "title LIKE 'Async%'"                # starts with "Async"
--where "title LIKE '%network%'"             # contains "network"
--where "title NOT LIKE '%Tutorial%'"        # excludes the word
--where "lower(title) LIKE '%rust%'"         # case-insensitive (via the lower() function)
```

#### Null filters

```bash
--where "author IS NOT NULL"                 # field must be set
--where "url IS NULL"                        # field must be absent
```

#### Array fields — auto-rewritten to `array_has(...)`

The `=` / `!=` / `IN` / `NOT IN` operators against an array field auto-rewrite to `array_has(...)` so element-containment "just works". The search output shows the rewrite as a one-line `Note` at the top.

```bash
--where "tags = 'rust'"                      # has 'rust' as one of its tags
--where "tags != 'archived'"                 # does NOT have 'archived' as a tag
--where "tags IN ('rust', 'python', 'go')"   # has at least one of these
--where "tags NOT IN ('archived', 'draft')"  # has NONE of these
--where "tags = 'rust' AND tags = 'async'"   # has BOTH (two array_has, AND'd)
--where "tags = 'rust' OR tags = 'python'"   # equivalent to IN, longer form
--where "array_has(tags, 'rust')"            # the explicit form — bypasses the rewrite
```

#### Date fields

Date literals use the `date '...'` keyword form. RFC 3339 datetimes use `timestamp '...'`.

```bash
--where "published > date '2024-01-01'"
--where "created BETWEEN date '2024-01-01' AND date '2024-12-31'"
--where "published >= date '2024-01-01' AND published < date '2025-01-01'"
```

#### Path filtering — the always-present `filepath` column

The `filepath` column stores the path relative to the project root — the **last component is the filename** (e.g. `articles/long-essay-2024.md` → filename is `long-essay-2024.md`).

```bash
--where "filepath LIKE 'articles/%'"         # everything under articles/
--where "filepath LIKE '%/notes/%'"          # any directory called notes/
--where "filepath LIKE '%-postmortem.md'"    # filename suffix
--where "filepath = 'articles/foo.md'"       # exact path
```

#### Combining filters — AND, OR, NOT, parentheses

```bash
--where "year > 2020 AND tags = 'rust'"
--where "year > 2020 AND (tags = 'rust' OR tags = 'python')"
--where "(author = 'Federica Bianchi' OR author = 'Lorenzo Conti') AND year > 2020"
--where "tags = 'rust' AND filepath LIKE 'technologies/%'"   # array + path
--where "concepts != 'CRDT' AND filepath LIKE 'technologies/%'"
```

#### Functions

Most DataFusion scalar functions work — handy when the raw value doesn't quite match.

```bash
--where "length(title) > 50"                 # long titles
--where "lower(author) LIKE '%bianchi%'"     # case-insensitive contains
--where "year + 1 > 2025"                    # arithmetic
```

Other internal columns exist (`start_line`, `end_line`, `built_at`, `chunk_text`) but are rarely useful for filtering — semantic / fulltext search handles those concerns better.

### `mdvs info`

Shows current config and index status: scan settings, field definitions, build metadata (model, chunk size, file counts). `-v` for full field detail.

### `mdvs clean`

Deletes `.mdvs/`. Doesn't touch `mdvs.toml`.

### `mdvs export-jsonschema`

Translates `[fields]` into a canonical JSON Schema 2020-12 document. Useful for sharing with other tools, or round-tripping through `mdvs init --from-jsonschema`.

- `--format json|toml` — output format (default `json`)
- `--output-file FILE` — write to file instead of stdout

### `mdvs scaffold`

Emits the artifacts that integrate mdvs with an agent harness (Claude Code, Codex, OpenCode, Cursor, Antigravity). Each subcommand prints to stdout; pipe it into the right location for your harness.

- `mdvs scaffold skill [--platform <name>]` — this skill file. Default destination: `.agents/skills/mdvs/SKILL.md` for Codex / OpenCode / Cursor / Antigravity; `.claude/skills/mdvs/SKILL.md` for Claude Code.
- `mdvs scaffold snippet [--platform <name>]` — the project-rules snippet for `AGENTS.md` / `CLAUDE.md` / `.cursor/rules/mdvs.mdc`.
- `mdvs scaffold hook --platform claude-code` — the `PostToolUse` hook config (a JSON snippet to merge into `.claude/settings.json`). The emitted snippet's `command:` fields call `mdvs hook handle` directly — no shell scripts, no `jq` dependency. **Currently the only verified hook integration.** The other platforms refuse with a pointer to their per-platform mdbook page; wiring `mdvs hook handle` into Codex / Cursor / OpenCode / Antigravity is possible by following each harness's own hooks documentation.

## Agent-harness integration

mdvs ships as a CLI; integrating it with an agent harness is wiring rather than installing. Three artifacts cover the three integration points:

| Artifact | Purpose | Coverage |
|---|---|---|
| **Skill file** (`SKILL.md`, this file) | Activated by harnesses implementing the [Agent Skills open standard](https://agentskills.io). Loaded on demand; agent reads procedure + reference. | Works in any harness that loads `.md` skills (Claude Code, Codex, Cursor, OpenCode, Antigravity, …). |
| **Project-rules snippet** | Always-on text in `AGENTS.md` / `CLAUDE.md` / `.cursor/rules/mdvs.mdc`. Short — names the KB, the search-vs-Grep preference, and the warning-loop rule. | Works in any harness that reads `AGENTS.md` / `CLAUDE.md` / `.cursor/rules`. |
| **`PostToolUse` hook** | Calls `mdvs hook handle` after every Edit / Write on a markdown file inside the vault. mdvs walks up to find `mdvs.toml`, runs `check`, and surfaces violations to you as **non-blocking** model-context. | **Shipped for Claude Code only.** Other harnesses: follow their documented hook system to wire `mdvs hook handle` in — see <https://edochi.github.io/mdvs/recipes/agent-harnesses/>. |

## Output format

Three formats; default is `pretty`.

- `--output pretty` — box-drawing tables for terminal display. Adapts to width.
- `--output markdown` — GFM tables and `##` headers. **Best for agent consumption** and the format the validation hook surfaces back through the harness's model-context channel.
- `--output json` — structured JSON for programmatic extraction (pipe through `jq` or any JSON tool).

Priority chain when `--output` is omitted: CLI flag > `default_output_format` in `mdvs.toml` > hard default (`pretty`). Same command always produces the same output regardless of TTY state. `-v` (verbose) adds per-step pipeline output with timings.

## Exit codes

- **0** — success (no violations)
- **1** — violations found (`check` and `build`)
- **2** — error (bad config, missing files, model mismatch)

Hook scripts use `|| true` to mask the exit-1 from `check` — the hook is intentionally non-blocking. Don't change that contract.

## Things to know

- Field types inferred automatically: `String`, `Integer`, `Float`, `Boolean`, `Date` (`YYYY-MM-DD`), `DateTime` (RFC 3339 with mandatory timezone), and `Array(<scalar>)` for any scalar type. The on-disk grammar is `Scalar | Array(Scalar)` only. Mixed scalar types widen (`Integer + String → String`).
- **Nested frontmatter uses dotted-name leaves.** `calibration.baseline.wavelength: 850.0` becomes a `[[fields.field]]` named `"calibration.baseline.wavelength"` of type `Float`. Top-level `Object` is rejected; nested `Array(Object{...})` is also rejected — represent arrays of structured items as parallel scalar arrays (e.g. `measurement_timestamps: Array(String)` + `measurement_values: Array(Float)`). SQL filters use dot notation: `--where "calibration.baseline.wavelength > 800"`.
- **Preprocessors opt into widening.** Each field carries a `preprocess` array. Built-ins: `coerce_to_string` (accepts non-string scalars on a `String` field) and `widen_int_to_float` (accepts integers on a `Float` field). Inference auto-populates these when widening was observed. `preprocess = []` means strict.
- Categorical detection is automatic for low-cardinality repeated values. Out-of-category values → `InvalidCategory`.
- Constraint kinds: `categories` (closed-set enum, mutually exclusive with everything else), `min`/`max` (numeric range), `min_length`/`max_length` (string and array length), `pattern` (regex on strings). Range / length / pattern are not auto-inferred; add manually or via `update reinfer <field> --with=range`.
- `init --force` rewrites the config from scratch. `update` preserves existing config and only adds new fields. `update reinfer` re-infers specific fields.
- Model identity is tracked: changing the model in `mdvs.toml` requires `build --force` to confirm a full re-embed.
- `check` auto-runs `update` first by default (unless `--no-update`, or `--jsonschema` is given, or `[check].auto_update = false` is set in `mdvs.toml`).
- `search` auto-runs `update` and `build` if needed (unless `--no-update` / `--no-build`).
- **`.mdvsignore` and `.gitignore` are both honored** when scanning. mdvs reuses the [`ignore`](https://crates.io/crates/ignore) crate (same matcher git uses), so the per-directory `.gitignore` rules you already have apply automatically. For mdvs-only exclusions (e.g. "don't index this draft directory, but keep it tracked in git"), drop a `.mdvsignore` next to the files using the same syntax as `.gitignore`. Both apply to `init` / `update` / `check` / `build` / `search`. To stop honoring `.gitignore` (for example, to index files that are deliberately untracked but should still be searchable), set `[scan].skip_gitignore = true` in `mdvs.toml`. Hidden files and dotfiles are NOT skipped — only `.md` / `.markdown` extensions are scanned to begin with.

## Examples

### Setting up a new vault from scratch

```bash
mdvs init                      # scan current directory, infer schema, write mdvs.toml
mdvs init ~/notes              # or point at another directory
mdvs init --dry-run            # preview without writing
```

### User added a new field to some files

```bash
mdvs check                     # → reports "category" as a new field (informational)
mdvs update                    # → detects "category", adds it to mdvs.toml
mdvs check                     # → clean
```

### Fixing violations after `check`

```bash
mdvs check
# →
#   MissingRequired: "title" missing in blog/drafts/untitled.md
#   WrongType: "priority" expected Integer, got String in projects/alpha.md
#   InvalidCategory: "status" got "wip", expected one of [draft, published, archived]
#   OutOfRange: "rating" got 11, expected min=1, max=5
```

Resolution per kind:
- **MissingRequired** — add the field, or remove the path from `required` in `mdvs.toml`
- **WrongType** — fix the value, or `mdvs update reinfer <field>` if the type should change
- **InvalidCategory** — fix the value, or `mdvs update reinfer <field> --with=categorical` to update the list
- **OutOfRange** — fix the value, or `mdvs update reinfer <field> --with=range` to widen bounds
- **Disallowed** — remove the field from that file, or widen `allowed` globs in `mdvs.toml`

### Example — responding to a hook-delivered violation

You wrote `status: in_review` to `kb/projects/alpha/sprint-12.md`. The next thing in your context is a hook-surfaced violation block (the exact channel name varies per harness; the content is the same):

```
## Violations

### InvalidCategory

- `kb/projects/alpha/sprint-12.md`: field `status` — expected one of `draft | published | archived`, got `in_review`
```

Your response:

> Just wrote `status: in_review` on `sprint-12.md`. mdvs's schema only allows `draft | published | archived` for `status`. Looking back, I've seen the `in_review` state come up in two other recent edits — this might be a real new lifecycle stage rather than a typo. Want me to add `in_review` to the schema (`mdvs update reinfer status --with=categorical` after updating the file with the new value), or should I change the value on `sprint-12.md` to one of the existing categories?

Note what you did NOT do: silently `mdvs update reinfer status --with=categorical` to make the warning go away. The user decides whether the schema or the file is the source of truth.

### Wiring mdvs into Claude Code (end-to-end)

```bash
mdvs scaffold skill > .claude/skills/mdvs/SKILL.md     # the skill file
mdvs scaffold snippet >> CLAUDE.md                     # the always-on rules block
mdvs scaffold hook --platform claude-code              # PostToolUse hook config — read stderr for the destination
```

`mdvs scaffold hook` prints a JSON snippet to merge into `.claude/settings.json`. No shell scripts; the `command:` fields call `mdvs hook handle` directly.

### Wiring mdvs into other harnesses (skill + snippet)

For Codex, Cursor, OpenCode, or Antigravity, install the skill and snippet — the hook half isn't shipped:

```bash
mdvs scaffold skill --platform <name> > <skills-path>/mdvs/SKILL.md
mdvs scaffold snippet --platform <name> >> <rules-file>
```

`mdvs scaffold hook --platform <name>` for these harnesses refuses with a pointer at <https://edochi.github.io/mdvs/recipes/agent-harnesses/>, which describes how to wire `mdvs hook handle` into each harness's own hook config using that harness's documentation.

### Searching with filters

Full reference for `--where` is in the [search reference section](#mdvs-search) above. A few realistic shapes:

```bash
# Recent articles by a specific author
mdvs search "actor model" --where "author = 'Federica Bianchi' AND year >= 2022"

# Notes tagged with both 'rust' and 'async' (two array_has, AND'd)
mdvs search "cancellation" --where "tags = 'rust' AND tags = 'async'"

# Anything published in 2024
mdvs search "consensus" --where "published BETWEEN date '2024-01-01' AND date '2024-12-31'"

# Restrict to a subtree, exclude archived
mdvs search "session model" --where "filepath LIKE 'projects/%' AND tags != 'archived'"

# Either of two authors, recent
mdvs search "types" --where "(author = 'Lorenzo Conti' OR author = 'Federica Bianchi') AND year > 2020"

# Verbose mode — shows the matching chunk text under each hit
mdvs search "calibration" -v
```

### Edge cases

- **Files without frontmatter (bare files):** `init` includes them by default. Use `--ignore-bare-files` or set `include_bare_files = false` in `[scan]` to exclude.
- **Null values:** `nullable = true` accepts null. Null skips type and category checks. A `required` + `nullable` field passes with `key: null` — fails only if the key is entirely absent.
- **Mixed-type fields:** widen to `String`. `1` becomes `"1"`. Intentional, not data loss.
- **Special characters in field names:** TOML handles quoting in `mdvs.toml`. In `--where`, wrap with double quotes: `--where "\"author's note\" IS NOT NULL"`.
- **Hook fires on non-vault edits:** if no `mdvs.toml` is reachable upward from the edited file, the hook exits silently. No false positives.
- **Hook stays silent on a bad edit:** check (in order) — harness matcher pattern, file extension (`.md`), `mdvs` on PATH for the harness's hook subprocess, symlinks outside the vault.

## Common errors

| Error | Cause | Fix |
|---|---|---|
| `mdvs.toml already exists` | Running `init` twice | `init --force` or `update` |
| `no markdown files found` | Wrong path or glob | Check path + `[scan].glob` in config |
| `model mismatch` | Config model differs from index | `build --force` to re-embed |
| `field 'X' is not in mdvs.toml` | `reinfer` on unknown field | Check spelling, or `update` first to add it |
| Violations on `check` | Frontmatter doesn't match schema | Read the list, fix files or evolve the schema (Step 4) |
| Hook stays silent on a bad edit | See Edge cases above | Check matcher, extension, PATH, vault location |
