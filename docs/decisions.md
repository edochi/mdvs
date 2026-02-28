# Design Decisions

Brainstorming notes and confirmed decisions, organized by topic.
Decisions here are **confirmed but not yet spec'd or implemented**.
Once a decision is implemented and reflected in the specs, remove it from this file.

---

## Command Design

### The three artifacts (Cargo analogy)

| mdvs | Cargo | Purpose | Git |
|------|-------|---------|-----|
| `mdvs.toml` | `Cargo.toml` | User-declared intent: schema, model, config | committed |
| `mdvs.lock` | `Cargo.lock` | Resolved state: exact model revision, file hashes, inferred fields | committed |
| `.mdvs/` | `target/` | Build artifacts (parquet files), regenerable from lock + source | gitignored |

**Why the lock exists separately from the parquets:**
The lock is the resolved inputs, like `Cargo.lock` pinning exact dependency versions.
The parquets are derived build outputs. You can clone a repo, see the lock, and know
exactly what should be indexed — without the parquet files. The lock is human-readable,
git-diffable, and provides staleness detection without reading binary parquet.

The information could technically live in parquet metadata, but:
- Lock is cheap to parse (TOML) vs parquet
- Lock is diffable in PRs ("these files changed, this field was added")
- Lock exists before build (after init/update, before parquets are generated)

### Two layers

The tool has two independent value propositions:

1. **Validation layer** — frontmatter schema enforcement. Needs only `mdvs.toml` +
   files on disk. No model, no embeddings, no parquets.
2. **Search layer** — semantic search. Needs model + parquets on top of the validation
   layer.

Commands split cleanly between layers:

| Layer | Commands |
|-------|----------|
| Validation | `init`, `update`, `check` |
| Search | `build`, `search` |
| Utility | `clean`, `info` |

The search layer depends on the validation layer (build needs toml + lock), but the
validation layer stands on its own.

### `init` — first-time setup (infer + update + build)

The "set up everything from scratch" command. Does the full pipeline:
1. Scan files
2. Infer schema — discover field names, types, allowed/required patterns
3. Download model
4. Write `mdvs.toml` (inferred schema as starting point for user to edit)
5. Write `mdvs.lock` (resolved state: exact model revision, file hashes, fields)
6. Build parquets (chunk, embed, write `files.parquet` + `chunks.parquet`)

`init` is an `update` with inference on top, followed by a `build`.

Flag: `--auto-build` (default true) — controls whether init triggers a build at
the end. Written to `mdvs.toml` as `auto_build` so update inherits the preference.

### `update` — re-scan, validate, refresh lock

Like `cargo update` — re-scan files, validate against schema, update the lock.

Requires `mdvs.toml` to exist (reads config from it).

**Steps:**
1. Scan files
2. **Pre-check** — validate all files against toml schema (types + paths)
3. Based on `on_error` behavior:
   - `fail` (default): validation errors block the update. User must fix files first.
   - `skip`: non-conforming files are excluded from the lock with a warning. Only
     clean files enter the lock (and therefore the index).
4. Update `mdvs.lock` with current file hashes and field metadata
5. If `auto_build` is true: trigger `build`

**Flags:**
- `--build=true|false` — override `auto_build` from toml (always wins)
- `--on-error=fail|skip` — override `on_error` from toml (always wins)
- `--infer=new|all|none` — how to handle new/changed fields:
  - `new` (default?) — infer only fields not already in toml
  - `all` — re-infer all fields (overwrite toml field definitions)
  - `none` — no inference; new fields get type String, allowed everywhere,
    required nowhere

**Toml config:**
```toml
[config]
auto_build = true    # update triggers build by default
on_error = "fail"    # validation errors block the update
```

### `check` — validate frontmatter against schema

Validate files on disk against the declared schema in `mdvs.toml`. Independent from
update — just scans files and reports violations. No side effects (doesn't modify
lock or parquets).

This is the same validation that `update` runs as a pre-check, but as a standalone
command for CI or manual inspection.

**Open questions:**
- What violations to report? Missing required fields, type mismatches, disallowed fields,
  unknown/undeclared fields?
- Output format: `file:field: message` lines? Exit 0 if clean, exit 1 if violations?
- Type matching leniency: does Float accept integer values? (Probably yes, since widening
  means some files had int and some had float.)

### `build` — rebuild the search index (expensive)

Read toml, scan files, chunk, embed, write parquets, update lock hashes. This is
where the model loads and the real work happens.

Use cases:
- After manually editing `mdvs.toml` (changing field types, model, etc.)
- After `update --build=false` to rebuild with refreshed lock
- Force rebuild

### `search` — query the index

Load model, embed query, run note-level SQL (MAX chunk similarity grouped by file),
print `score  filename` to stdout.

Flags: `--limit` (default 10), `--path`, `--where` (SQL WHERE clause).
Revision mismatch is a warning (not error).

### `clean` — remove `.mdvs/` directory

Delete build artifacts. Like `cargo clean`.

### `info` — show index status

Display model name/revision, file count, chunk count, staleness (files changed since
last build).

### Typical workflows

```
# First time:
mdvs init                          # scan, infer, build (auto_build=true)

# Files changed on disk:
mdvs update                        # validate, refresh lock, auto-build
mdvs update --build=false          # validate, refresh lock only (review diff first)

# Edited toml manually:
mdvs build                         # rebuild with new config

# CI/validation:
mdvs check                         # validate frontmatter, exit 1 on violations

# Lenient update (skip broken files):
mdvs update --on-error=skip        # warn about bad files, exclude them, build
```

---

## mfv

### Future validation features (post-v0.3)

Additional optional constraints on `[[fields.field]]`:
- `min` / `max` for integers and floats (bounds)
- `after` / `before` for dates (date ranges)
- `values` for string enums (already spec'd)
- `pattern` for regex matching (already spec'd)
- `min_length` / `max_length` for strings/arrays

---

## mdvs

### `search` vs `query` commands

Two separate commands, explicit intent:
- `mdvs search "query text"` — always involves vector similarity
- `mdvs query "SELECT ..."` — pure SQL on metadata, no vectors

No auto-detection or magic parsing.

### SQL flags on `search`

| Flag | Maps to | Example |
|------|---------|---------|
| `--where` | WHERE | `--where "tags LIKE '%rust%'"` |
| `--select` | SELECT (extra columns) | `--select "tags, category"` |
| `--order` | ORDER BY (secondary sort) | `--order "date DESC"` |
| `--limit` | LIMIT | `--limit 20` |

**`--where` filters BEFORE vector ranking** — filter first, then compute
embeddings/distances only on surviving rows. Cheaper and more intuitive.

No `--group-by` for now.

### Storage: `.mdvs/` directory with two Parquet files

The artifact is the `.mdvs/` directory (like `target/` in cargo). Contains:
- `files.parquet` — one row per file: `file_id` (UUID), `filename`, `frontmatter` (JSON), `content_hash`, `built_at`
- `chunks.parquet` — one row per chunk: `chunk_id` (UUID), `file_id` (FK), `chunk_index`, `start_line`, `end_line`, `embedding`

No dynamic field columns — all frontmatter lives in a single JSON column.
Simpler schema, no rebuild when field config changes.

No bundling/compression of multiple Parquets together. Parquet already compresses
well internally (zstd per column chunk). Any archive format would kill random access
and memory-mapping, defeating columnar scanning.

No raw markdown text stored in either file — keep it lightweight. Search results
show file path + score. User opens the original file for content. `--snippets` flag
reads chunk text from file using line offsets.

Model change requires re-reading all files from disk (no cached plain_text).

### Chunk hashing for incremental re-embedding (deferred to v0.4+)

Each chunk gets a content hash. On rebuild:
1. Re-chunk the file (fast, pure text processing)
2. Hash each chunk
3. Compare against stored chunk hashes in `chunks.parquet`
4. Only embed chunks with new/changed hashes
5. Reuse existing embeddings for unchanged chunks

Benefit: adding a paragraph to a long document re-embeds only 1-2 chunks instead of all.
Requires deterministic chunking — `text-splitter` MarkdownSplitter splits on structural
boundaries (headers, paragraphs), so unchanged sections produce identical chunks even
with mid-file edits.

v0.3: file-level hashes only (in `mdvs.lock`). File changed → full re-chunk + re-embed.

### `describe` command (post-v0.3)

A command that, given a subpath, shows the shape of the data:
- For each meaningful subpath (only those carrying information, not redundant subpaths):
  which fields are allowed and required
- For each field: useful characteristics for query planning — upper/lower bounds,
  statistical metrics (mean, variance, etc.)
- Distinction between validation boundaries (configured constraints like "no values < 0")
  and observed boundaries (actual data extremes like "smallest value is 1")
- Only meaningful subpaths shown (same logic as inference.rs tree — if field is required
  in `folder/**`, don't repeat it for `folder/subfolder/**`)

Split between mfv and mdvs:
- mfv: schema view (allowed/required per subpath, configured constraints)
- mdvs: adds data statistics on top (observed ranges, means, etc.)

Requires setters/getters for validation boundaries per field (min/max, after/before, etc.)
— ties into "Future validation features" above.
