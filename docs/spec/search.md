# Search

Deep-dive into the search pipeline. For the module map see [architecture.md](./architecture.md).

Search delegates to LanceDB. mdvs's role is: translate the query and the optional `--where` clause into a LanceDB query, dispatch on `SearchMode`, deduplicate to the best chunk per file. Key files: `search.rs` (mode enum, score-column resolution), `index/backend.rs` (`LanceBackend::search`, the `--where` translator).

## SearchMode

`SearchMode` (`search.rs`):

| Variant | Score column on result rows | What runs |
|---|---|---|
| `Semantic` | `_distance` (mapped to `1 - d`) | `.nearest_to(query_embedding).distance_type(Cosine)` |
| `Fulltext` | `_score` | `.full_text_search(FullTextSearchQuery::new(query))` |
| `Hybrid` (default) | `_relevance_score` | both of the above + `.rerank(RrfReranker::default())` |

The CLI flag `--mode {semantic,fulltext,hybrid}` selects the variant; the default is `Hybrid`. `Semantic` and `Hybrid` require the embedding model to be loaded; `Fulltext` does not.

## --where translation

`translate_where_to_struct` (`index/backend.rs`) rewrites the user clause so that:

- Bare frontmatter field names get a `data.` prefix (so `status = 'active'` becomes `data.status = 'active'`).
- Identifiers immediately followed by `(` are treated as **function calls**, not field names — `lower(status)` is rewritten to `lower(data.status)`, not `data.lower(...)`.
- Internal columns (`chunk_text`, `start_line`, `end_line`, `embedding`, …) are left bare.
- **References to `Array(Float)` field names produce an early error** with a clear message, before LanceDB sees the clause. This is the TODO-0159 mitigation — see the upstream draft at `docs/spec/todos/TODO-0159-upstream-draft.md`.
- Date and timestamp literal keywords (`DATE '...'`, `TIMESTAMP '...'`) are protected from prefix injection by a literal-aware tokenizer.

The translator is schema-aware: it loads the `data` Struct's child names + types from the Lance table schema once per `search()` call, via `float_list_child_names(schema)` for the Array(Float) guard and the full child-name set for prefixing.

## Query execution (`LanceBackend::search`)

`index/backend.rs::LanceBackend::search(mode, query, query_embedding, where_clause, limit)`:

1. **Open the table** — `conn.open_table("index")`.
2. **Build the query** — `table.query()` plus the mode-specific clauses listed in the `SearchMode` table above.
3. **Filter** — `.only_if(<translated where>)` if `where_clause` is `Some`.
4. **Over-fetch** — `.limit(limit.saturating_mul(OVER_FETCH_FACTOR))` with `OVER_FETCH_FACTOR = 3`. This compensates for chunks that will be dropped by the best-chunk-per-file dedupe step.
5. **Stream** — `.execute().await?` yields a `RecordBatchStream`; `try_collect()` materialises all batches.
6. **Limit-zero short circuit** — if the caller asked for `limit == 0` we return `Ok(vec![])` before reaching LanceDB, so the user sees no results instead of a cryptic "k must be positive" error.

## Score column resolution

Each mode produces a different score column on the result rows. `resolve_score_column(mode)` (`search.rs`) returns the constant column name; for `Semantic` the raw value is `_distance` (smaller = closer), which the result-reader maps to `1.0 - d` so callers see "higher is better" uniformly across modes.

## Best-chunk-per-file dedupe

Results come back per-chunk. mdvs collapses them in Rust:

1. Iterate the streamed rows in their LanceDB-returned order (already ranked by the mode's score).
2. Insert into a `HashMap<file_id, SearchHit>`, keeping the highest-scored chunk per file.
3. Sort the resulting hits by score descending and truncate to `--limit`.

The over-fetch factor (×3) ensures that even when many of the top-ranked chunks come from a single file, we still have enough candidates from other files to fill the requested limit.

## Verbose snippet

In verbose mode `cmd/search.rs` reads the best chunk's text directly from the `chunk_text` column on the winning row. No second file read is needed — `chunk_text` is persisted on the index for exactly this purpose (and for the FTS index to operate on).

## Result Assembly

`SearchHit` (`index/backend.rs`):

```rust
pub struct SearchHit {
    pub filename: String,             // from filepath column
    pub score: f64,                   // mode-dependent (see SearchMode table)
    pub start_line: Option<i32>,      // best chunk's start (1-based)
    pub end_line: Option<i32>,        // best chunk's end (1-based, inclusive)
    pub chunk_text: Option<String>,   // from the chunk_text column (always populated in verbose mode)
}
```

Assembled by downcasting LanceDB-returned Arrow arrays: `StringArray` for `filepath`/`chunk_text`, `Float64Array` for the score column, `Int32Array` for line ranges.

## Collision Avoidance

Internal columns (`chunk_id`, `file_id`, `chunk_index`, `start_line`, `end_line`, `chunk_text`, `embedding`, `filepath`, `content_hash`, `built_at`) live at the top level of the schema. Frontmatter fields live under the `data` Struct. The `--where` translator rewrites bare frontmatter names to `data.<name>`, so the *qualified* SQL paths never clash even when a frontmatter field shares its name with an internal column.

The user-visible question is what a bare reference in `--where` *means*. By default `filepath` in a `--where` clause refers to the internal column. If the user has a frontmatter field also called `filepath`, that's a collision the user has to resolve — the translator detects it and bails with an actionable error.

Resolution at the translator layer uses `[search].internal_prefix` and `[search.aliases]`:

- **Prefix** — `internal_prefix = "_"` renames the *bare reference* for all internal columns: the user writes `_filepath` to refer to the internal column, and `filepath` stays bare for the frontmatter field (translated to `data.filepath`).
- **Alias** — `[search.aliases].filepath = "path"` renames the *bare reference* for one internal column: the user writes `path` for the internal column and `filepath` stays bare for the frontmatter field.

These only affect `--where` translation; the actual on-disk column names are always the literal constants from `index/storage.rs`. The translator handles the mapping in `translate_where_to_struct` (`index/backend.rs`).
