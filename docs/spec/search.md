# Search

Deep-dive into the search pipeline. For the module map see [architecture.md](./architecture.md).

Search wires DataFusion, a custom cosine similarity UDF, and a view that promotes frontmatter fields for bare-name SQL access. Key files: `search.rs` (context + UDF), `index/backend.rs` (query execution + result assembly).

## SearchContext

`SearchContext` at `search.rs:135` wraps a DataFusion `SessionContext` with registered tables, a cosine UDF, and a `files_v` view.

### Initialization (`SearchContext::new()` at `search.rs:139`)

1. **Register parquet tables** — `files` and `chunks` from `.mdvs/`
2. **Register UDF** — `CosineSimilarityUDF` capturing the query embedding vector
3. **Read data Struct schema** — extract child field names from the `data` column in `files` table. These are the frontmatter fields that will be promoted.
4. **Resolve internal column names** — for each internal column (`file_id`, `filepath`, `content_hash`, `built_at`), apply `resolve_view_name(col, prefix, aliases)`:
   - If alias exists for this column → use alias
   - Else if `internal_prefix` is non-empty → use `prefix + col`
   - Else → use raw column name
5. **Collision detection** — if any resolved internal name matches a frontmatter field name, bail with an error suggesting prefix or alias resolution
6. **Create `files_v` view** — SQL view that promotes `data` Struct children to top-level columns:
   ```sql
   CREATE VIEW files_v AS
   SELECT _file_id, _filepath, _content_hash, _built_at, data,
          data['title'] AS "title",
          data['status'] AS "status",
          ...
   FROM files
   ```
   Special character escaping: single quotes doubled in accessor (`data['author''s_note']`), double quotes doubled in alias (`AS "author""s_note"`).

## Cosine Similarity UDF

`CosineSimilarityUDF` at `search.rs:20` implements DataFusion's `ScalarUDFImpl`:

- **Input**: `FixedSizeList<Float32>` (chunk embeddings column)
- **Output**: `Float64` (similarity score)
- **Captured state**: query vector (`Vec<f32>`) — fixed at UDF creation, same for all rows

### Computation (`invoke_with_args()`)

For each row in the embeddings column:
1. If row is null → return NULL
2. Extract float array from FixedSizeList
3. Compute: `dot = Σ(embed[j] * query[j])`, `row_norm = √(Σ embed[j]²)`
4. Query norm precomputed once: `query_norm = √(Σ query[j]²)`
5. If either norm is 0.0 → return 0.0 (avoids NaN)
6. Else → return `dot / (query_norm * row_norm)` as f64

Result range: [-1.0, 1.0] for normalized vectors.

## Query Structure

Generated SQL in `ParquetBackend::search()` at `index/backend.rs`:

```sql
SELECT f."_filepath",
       sub.score,
       sub.start_line,
       sub.end_line
FROM (
    SELECT c.file_id,
           cosine_similarity(c.embedding) AS score,
           c.start_line,
           c.end_line,
           ROW_NUMBER() OVER (
               PARTITION BY c.file_id
               ORDER BY cosine_similarity(c.embedding) DESC
           ) AS rn
    FROM chunks c
) sub
JOIN files_v f ON sub.file_id = f."_file_id"
WHERE sub.rn = 1
  [AND <user_where_clause>]
ORDER BY sub.score DESC
LIMIT <limit>
```

### Key design points

**Note-level ranking** — `ROW_NUMBER() OVER (PARTITION BY file_id ORDER BY score DESC)` with `WHERE rn = 1` selects the best chunk per file. Results are per-file, scored by their best chunk — not average. Rationale: a file with one highly relevant paragraph should rank above a file with many mediocre ones.

**WHERE clause injection** — the user's `--where` clause is appended with `AND` after `sub.rn = 1`. It operates on `files_v` columns, so bare frontmatter field names work (`status = 'draft'`). No sanitization beyond DataFusion's SQL parser — the clause is passed as-is.

**Verbose mode** — when verbose, the `cmd/search.rs` command reads the best chunk's text from the source file using `start_line`/`end_line`. This is a separate file read, not from Parquet.

## Result Assembly

`SearchHit` at `index/backend.rs:18`:

```rust
pub struct SearchHit {
    pub filename: String,           // from files_v._filepath
    pub score: f64,                 // from cosine_similarity()
    pub start_line: Option<i32>,    // best chunk's start (1-based)
    pub end_line: Option<i32>,      // best chunk's end (1-based, inclusive)
    pub chunk_text: Option<String>, // populated later by cmd/search.rs in verbose mode
}
```

Assembled by downcasting DataFusion query result columns: StringViewArray (filenames), Float64Array (scores), Int32Array (lines). `chunk_text` is always `None` from the backend — filled by the command layer.

## Collision Avoidance

Problem: a frontmatter field named `filepath` collides with the internal `_filepath` column.

Detection: `SearchContext::new()` checks if any resolved internal column name matches a frontmatter field name. Bails with an actionable error message.

Resolution (three tiers):
1. **Prefix** — `[search].internal_prefix = "_"` renames all internal columns: `file_id` → `_file_id`
2. **Alias** — `[search.aliases].filepath = "file_path"` renames one column
3. **Rename** — change the frontmatter field name (application-level)

Prefix is applied by default (empty string = no prefix). Aliases take precedence over prefix.
