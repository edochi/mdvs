# Workflow: Search

**Status: DRAFT**

**Cross-references:** [Terminology](../01-terminology.md) | [Crate: mdvs](../10-crates/mdvs/spec.md) | [Database Schema](../20-database/schema.md)

---

## Overview

The search workflow embeds a user query, performs cosine distance search against the HNSW index, and returns results ranked at the note level (or chunk level with `--chunks`).

---

## Actors

| Actor | Role |
|---|---|
| **User** | Provides query string and optional filters |
| **CLI** | Orchestrates the search |
| **model2vec-rs** | Embeds the query string |
| **DuckDB** | HNSW search, JOIN, ranking |

---

## Sequence

```mermaid
sequenceDiagram
    participant U as User
    participant CLI as mdvs CLI
    participant DB as DuckDB
    participant M as model2vec-rs

    U->>CLI: mdvs search "query" [--where ...] [-n N]

    %% Model identity check
    CLI->>DB: Load model identity from vault_meta
    CLI->>M: Load model
    CLI->>CLI: Check model identity (warning on revision mismatch)

    %% Staleness check
    alt on_search = "auto"
        CLI->>CLI: Run incremental index (see index.md)
    else on_search = "strict"
        CLI->>CLI: Check for stale files, error if any
    end

    %% Embed query
    CLI->>M: embed("query")
    M-->>CLI: query_embedding Vec<f32>

    %% Search
    alt Note-level (default)
        CLI->>DB: Note-level ranked query (see below)
        DB-->>CLI: Results: filename, promoted fields, distance, heading, snippet
    else Chunk-level (--chunks)
        CLI->>DB: Chunk-level query (see below)
        DB-->>CLI: Results: chunk_id, filename, heading, snippet, distance
    end

    %% Format output
    alt --format table (default)
        CLI->>U: Formatted table with rank, filename, heading, distance, metadata, snippet
    else --format json
        CLI->>U: JSON array of result objects
    else --format paths
        CLI->>U: One filename per line
    end
```

---

## Note-Level Ranking

Default mode. Groups chunk results by file and ranks by best chunk match.

**Strategy:**

- **Score:** Maximum similarity (minimum cosine distance) across all chunks of a file
- **Snippet:** Plain text of the best-matching chunk (truncated to `snippet_length`)
- **Heading:** The heading associated with the best-matching chunk

```sql
WITH ranked_chunks AS (
    SELECT
        c.filename,
        c.heading,
        LEFT(c.plain_text, :snippet_length) AS snippet,
        array_cosine_distance(c.embedding, :query_vec::FLOAT[N]) AS distance
    FROM chunks c
)
SELECT
    m.filename,
    -- [dynamic promoted columns from vault_meta.promoted_fields]
    MIN(rc.distance) AS distance,
    FIRST(rc.heading ORDER BY rc.distance) AS best_heading,
    FIRST(rc.snippet ORDER BY rc.distance) AS snippet
FROM ranked_chunks rc
JOIN mdfiles m ON rc.filename = m.filename
-- [optional: WHERE {user_provided_clause}]
GROUP BY m.filename -- [, dynamic promoted columns]
ORDER BY distance
LIMIT :limit;
```

The `--where` clause is injected into the `WHERE` position, operating on `mdfiles` columns (both promoted and `metadata` JSON). This gives users the full power of DuckDB SQL for filtering.

---

## Chunk-Level Mode (`--chunks`)

Bypasses note-level grouping. Returns individual chunks ranked by similarity.

```sql
SELECT
    c.chunk_id,
    c.filename,
    c.heading,
    LEFT(c.plain_text, :snippet_length) AS snippet,
    array_cosine_distance(c.embedding, :query_vec::FLOAT[N]) AS distance
FROM chunks c
JOIN mdfiles m ON c.filename = m.filename
-- [optional: WHERE {user_provided_clause}]
ORDER BY distance
LIMIT :limit;
```

Useful for finding specific sections across different files, or when a single long file has multiple relevant sections.

---

## Output Formats

### Table (default)

```
── Results for "how does CRDT conflict resolution work" ──

 1. projects/collabide/crdt-design.md § Conflict Resolution    0.142
    [rust, crdt, collaborative]  2025-06-12
    Operational Transform vs CRDT approaches for the editor...

 2. reading/kleppmann-crdt-paper.md § Summary                  0.198
    [papers, distributed-systems]  2025-03-20
    Notes on Martin Kleppmann's paper on conflict-free...

2 results (8ms search, 1ms embed)
```

**Line 1:** Rank, filename, `§ heading` (if present), cosine distance.
**Line 2:** Promoted field values (tags, date, etc.).
**Line 3:** Snippet from the best-matching chunk.
**Footer:** Result count, search time, embedding time.

### JSON

```json
{
  "query": "how does CRDT conflict resolution work",
  "results": [
    {
      "rank": 1,
      "filename": "projects/collabide/crdt-design.md",
      "heading": "Conflict Resolution",
      "distance": 0.142,
      "snippet": "Operational Transform vs CRDT approaches for the editor...",
      "title": "CRDT Design Notes",
      "tags": ["rust", "crdt", "collaborative"],
      "date": "2025-06-12"
    }
  ],
  "timing": {
    "embed_ms": 1,
    "search_ms": 8
  }
}
```

Promoted fields are included as top-level keys in each result object. The field names depend on what was promoted at init.

### Paths

```
projects/collabide/crdt-design.md
reading/kleppmann-crdt-paper.md
```

One filename per line. Useful for piping into other tools (`xargs`, `fzf`, editors).

---

## Filter Examples

Filters use SQL expressions directly against `mdfiles` columns:

```bash
# Filter by promoted array column
mdvs search "crdt resolution" --where "tags @> ['rust']"

# Filter by promoted date column
mdvs search "authentication" --where "date > '2025-01-01'"

# Filter by non-promoted field via JSON metadata
mdvs search "deployment" --where "metadata->>'author' = 'edoardo'"

# Combine filters
mdvs search "testing" --where "tags @> ['rust'] AND date > '2024-01-01'"
```

---

## Edge Cases

| Case | Behavior |
|---|---|
| Empty query string | Error: query must not be empty |
| No results found | Exit code 1, message: "No results found." |
| `--where` with syntax error | DuckDB SQL error, surfaced to user with the invalid clause highlighted |
| `--where` referencing non-existent column | DuckDB error, surfaced to user |
| Database not initialized | Error: ".mdvs.duckdb not found. Run `mdvs init` first." |
| Empty index (init done, no index yet) | No results (chunks table is empty) |
| Model mismatch | See [Model Mismatch Workflow](model-mismatch.md) |

---

## Related Documents

- [Terminology](../01-terminology.md) — definitions for note-level ranking, embedding, HNSW
- [Crate: mdvs](../10-crates/mdvs/spec.md) — search implementation
- [Database Schema](../20-database/schema.md) — query patterns
- [Workflow: Model Mismatch](model-mismatch.md) — identity check before search
- [Workflow: Index](index.md) — auto-sync in `on_search = "auto"` mode
