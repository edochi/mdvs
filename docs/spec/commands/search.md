# `mdvs search`

Embed a query and find the most similar files via cosine similarity.

## Pipeline

`cmd/search.rs` → `run()`

1. **Read config** — `MdvsToml::read()` + `validate()`
2. **Auto-update + auto-build** — chains update → build if configured (respects `[search].auto_update` and `[search].auto_build`)
3. **Read index metadata** — `Backend::read_build_metadata()`. Hard error if model in config differs from model in parquet metadata.
4. **Load model** — `Embedder::load()` from config
5. **Embed query** — `embedder.embed(&query)` → `Vec<f32>`
6. **Execute search** — `Backend::search(query_embedding, where_clause, limit, prefix, aliases)`:
   - Creates `SearchContext` (registers tables, UDF, `files_v` view)
   - Generates SQL: subquery ranks chunks per file, JOIN with `files_v`, apply `--where`, ORDER BY score DESC, LIMIT
   - Returns `Vec<SearchHit>`
7. **Fetch chunk text** — in verbose mode, reads source file lines for each hit's `start_line..end_line`

Returns `SearchOutcome` with `query`, `hits: Vec<SearchHit>`, `model_name`, `limit`.

## Key points

- **Model mismatch → hard error** — search refuses to run if the model in config differs from what built the index.
- **Note-level ranking** — results are per-file, scored by the best chunk's cosine similarity (max, not average).
- **`--where` injection** — raw SQL appended to the query. Operates on `files_v` view columns (bare frontmatter names work).

See [search.md](../search.md) for SearchContext, cosine UDF, and SQL structure details.
