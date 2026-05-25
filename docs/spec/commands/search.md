# `mdvs search`

Query the Lance index via LanceDB — semantic (cosine), full-text (BM25), or hybrid (RRF reranker over both).

## Pipeline

`cmd/search.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`
2. **Auto-update + auto-build** — chains update → build if configured (respects `[search].auto_update` and `[search].auto_build`)
3. **Read index metadata** — `LanceBackend::read_metadata()`. Hard error if the model in config differs from the model stored on the Lance table metadata.
4. **Load model** — `Embedder::load()` from config. Skipped for `--mode fulltext`.
5. **Embed query** — `embedder.embed(&query)` → `Vec<f32>`. Skipped for `--mode fulltext`.
6. **Execute search** — `LanceBackend::search(mode, query, query_embedding, where_clause, limit)`:
   - Translates `--where` via `translate_where_to_struct` (bare frontmatter names → `data.*`; scalar function calls left as-is; references to `Array(Float)` fields rejected — see TODO-0159).
   - Builds the LanceDB query for the selected `SearchMode`:
     - `Semantic` → `.nearest_to(query_embedding).distance_type(Cosine)`
     - `Fulltext` → `.full_text_search(FullTextSearchQuery::new(query))`
     - `Hybrid` (default) → both of the above + `.rerank(RrfReranker::default())`
   - Applies `.only_if(<translated where>)` if a filter was given and `.limit(limit × OVER_FETCH_FACTOR)` (`OVER_FETCH_FACTOR = 3`).
   - Streams chunk rows back; reads the per-mode score column (`_distance` mapped to `1 - d`, `_score`, or `_relevance_score`).
7. **Best-chunk-per-file dedupe** — group by `file_id`, keep the highest-scored chunk per file, trim to `--limit`.
8. **Verbose snippet** — read directly from the persisted `chunk_text` column on the winning row (no second file read).

Returns `SearchOutcome` with `query`, `mode`, `hits: Vec<SearchHit>`, `model_name`, `limit`.

## Key points

- **Model mismatch → hard error** — applies to semantic and hybrid modes; fulltext doesn't load the model and is unaffected.
- **Note-level ranking** — results are per-file, scored by the best chunk (max, not average).
- **`--where` translation** — bare frontmatter names get a `data.` prefix, scalar function calls (`lower(...)`, `length(...)`, `abs(...)`, …) are left untouched, and `Array(Float)` field references produce an early error before LanceDB sees them. Dot-notation works for nested-leaf fields (`WHERE data.calibration.baseline.wavelength > 800`).
- **No vector index below 10k chunks** — `VECTOR_INDEX_MIN_ROWS = 10_000` gates IVF-PQ. Smaller vaults run an exact flat scan inside LanceDB, which is plenty fast at that scale.

See [search.md](../search.md) for the `SearchMode` dispatch, score-column resolution, and dedupe details.
