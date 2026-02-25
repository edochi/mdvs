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
- `files.parquet` — one row per file: path, content_hash, frontmatter columns
- `chunks.parquet` — one row per chunk: file_id, chunk_index, chunk_hash, byte_offset_start, byte_offset_end, embedding

No bundling/compression of multiple Parquets together. Parquet already compresses
well internally (zstd per column chunk). Any archive format would kill random access
and memory-mapping, defeating columnar scanning.

No raw markdown text stored in either file — keep it lightweight. Search results
show file path + score + frontmatter fields. User opens the original file for content.

### Chunk hashing for incremental re-embedding

Each chunk gets a content hash. On rebuild:
1. Re-chunk the file (fast, pure text processing)
2. Hash each chunk
3. Compare against stored chunk hashes in `chunks.parquet`
4. Only embed chunks with new/changed hashes
5. Reuse existing embeddings for unchanged chunks

Benefit: adding a paragraph to a long document re-embeds only 1-2 chunks instead of all.
Requires deterministic chunking (`text-splitter` is algorithmic, not stochastic).

### Chunk byte offsets

Store `byte_offset_start` and `byte_offset_end` in `chunks.parquet`.
Enables extracting the matching section from the original file at display time.
Cheap to store, useful for v0.4 polish (showing which part of a file matched).
