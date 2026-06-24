# Search & Indexing

mdvs builds a search index by chunking your markdown content, embedding it with a local model, and storing chunks + vectors + frontmatter in a single [LanceDB](https://lancedb.com/) dataset. Queries are served by LanceDB natively — semantic (vector), full-text (BM25), or hybrid (both, reranked) — with optional SQL filtering on frontmatter fields.

## Building the index

`mdvs build` (or `mdvs init` with auto-build) creates the search index in three steps: chunk, embed, store.

### Chunking

Each file's markdown body is split into semantic chunks — respecting headings, paragraphs, and code blocks rather than cutting at arbitrary character boundaries. The maximum chunk size is configurable (default 1024 characters) via the `[chunking]` section in `mdvs.toml`:

```toml
[chunking]
max_chunk_size = 1024
```

Each chunk tracks its start and end line numbers in the original file, so search results can point to the exact location.

### Embedding

Chunks are embedded into dense vectors using a local [Model2Vec](https://minish.ai/packages/model2vec/introduction) model by [Minish](https://minish.ai/) — static embeddings that run on CPU with no external services or GPU required. The model is downloaded from [HuggingFace](https://huggingface.co/minishlab) to the local cache on first use.

```toml
[embedding_model]
provider = "model2vec"
name = "minishlab/potion-multilingual-128M"
```

The default is `potion-multilingual-128M` — 101 languages, ~480 MB on disk. The full [POTION family](https://huggingface.co/collections/minishlab/potion-6721e0abd4ea41881417f062):

| Model | Parameters | Notes |
|---|---|---|
| `minishlab/potion-base-2M` | 2M | Smallest, fastest |
| `minishlab/potion-base-8M` | 8M | English-only, ~60 MB — good balance for English vaults |
| `minishlab/potion-base-32M` | 32M | English-only, higher quality, slower |
| `minishlab/potion-retrieval-32M` | 32M | English-only, optimized for retrieval tasks |
| `minishlab/potion-multilingual-128M` | 128M | Default — 101 languages |

Any Model2Vec-compatible model on HuggingFace works — set the `name` to its model ID. You can pin a specific revision for reproducibility.

### Storage

A single Lance dataset is written to `.mdvs/index.lance/` — **one row per chunk**, with everything you need on the same row:

| Column | Purpose |
|---|---|
| `chunk_id`, `file_id`, `chunk_index`, `start_line`, `end_line` | Chunk identity and source location |
| `chunk_text` | The plain-text chunk body — used by the full-text index and shown as the snippet in verbose results |
| `embedding` | Dense vector for semantic search (`FixedSizeList<Float32>`) |
| `filepath`, `content_hash`, `built_at` | Per-file metadata (duplicated on each of that file's chunks) |
| `data` | Frontmatter as an Arrow Struct (nested for dotted-name fields) — this is what `--where` filters query against |

Inside the dataset, two indexes are built at `mdvs build` time:

- A **full-text BM25 index** on `chunk_text`, always built.
- A **cosine IVF-PQ vector index** on `embedding`, only built when the index has at least ~10,000 chunks. Smaller vaults use LanceDB's exact flat scan, which is plenty fast at that scale.

## Incremental builds

Build only re-embeds what changed. Each file's markdown body (excluding frontmatter) is hashed, and the hash is compared against the existing index:

| Classification | Condition | Action |
|---|---|---|
| **New** | File not in index | Chunk, embed, add |
| **Edited** | Hash changed | Re-chunk, re-embed, replace chunks |
| **Unchanged** | Hash matches | Keep existing chunks |
| **Removed** | In index but not on disk | Drop file and its chunks |

Frontmatter-only changes (adding a tag, fixing a typo in `author`) rewrite the `data` column on every chunk row without re-embedding — the body hash hasn't changed, so the vectors are still valid.

When nothing needs embedding, the model isn't even loaded. When the change set is also empty (no new, edited, or removed files), the index write itself is skipped — `mdvs build` on an unchanged corpus does no Lance work at all. A `--force` flag bypasses both skips and triggers a full overwrite regardless of hashes. The non-force path that does need to persist a change is incremental: the rows for new, edited, and removed files are deleted and the freshly embedded chunks are appended, avoiding a full table rewrite.

## How search works

When you run `mdvs search "query" example_kb`, LanceDB does the heavy lifting. The shape of the work depends on `--mode` (default `hybrid`):

- **`semantic`** — the query is embedded with the same model used during build, and chunks are ranked by cosine similarity against `embedding`. Up to ~10,000 chunks, LanceDB does an exact flat scan; above that, the IVF-PQ vector index narrows the candidate set first.
- **`fulltext`** — the query is tokenized and scored against the BM25 full-text index on `chunk_text`. No model load needed.
- **`hybrid`** — both of the above run in parallel and their result lists are combined by LanceDB's Reciprocal Rank Fusion reranker. Default mode because it tolerates queries that are either keyword-y or fuzzy.

For guidance on which mode to reach for, see [Search Modes](./search-modes.md).

After LanceDB returns ranked chunk rows, mdvs deduplicates to the **best chunk per file** (a file with one highly relevant section ranks above a file with uniformly mediocre content) and then trims to `--limit` (default 10). LanceDB is asked for `limit × 3` candidates to make sure dedupe has enough material to work with.

### Scores

The score column in search output depends on the mode:

- **Semantic** — cosine similarity, a value in roughly `[0, 1]` (higher = more similar).
- **Fulltext** — BM25 relevance score, unbounded above (higher = better match).
- **Hybrid** — RRF score, also unbounded above.

Scores depend on the mode, the model, and the content, so there's no universal threshold for "relevant." Compare scores relative to each other within a single query.

## Filtering with `--where`

Add a SQL filter to narrow results by frontmatter fields:

```bash
mdvs search "calibration" example_kb --where "status = 'active'"
```

The `--where` clause filters on frontmatter fields — only chunks whose file matches the filter are included in the results. The filter and similarity ranking are combined in a single LanceDB query, so non-matching rows are excluded efficiently.

You can use any SQL expression that LanceDB's filter supports:

```bash
--where "draft = false"
--where "status = 'active' AND author = 'Giulia Ferretti'"
--where "sample_count > 10"
```

Array fields, nested objects, and field names with special characters require specific syntax — see the [Search Guide](../search-guide.md) for the full reference.

## Model identity

Search refuses to run if the model configured in `mdvs.toml` doesn't match the model that was used to build the index. This is a hard error, not a warning.

Embeddings from different models are incompatible — cosine similarity between vectors from different models produces meaningless scores. If you change the model, rebuild the index with `mdvs build --force`.
