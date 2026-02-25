# Design Decisions

Brainstorming notes and confirmed decisions, organized by topic.
Decisions here are **confirmed but not yet spec'd or implemented**.
Once a decision is implemented and reflected in the specs, remove it from this file.

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
