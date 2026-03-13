# Search & Indexing

mdvs builds a search index by chunking your markdown content, embedding it with a local model, and storing everything in Parquet files. Queries are embedded with the same model and ranked by cosine similarity, with optional SQL filtering on frontmatter fields.

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
name = "minishlab/potion-base-8M"
```

The default is `potion-base-8M`, a good balance of size and quality. The full [POTION family](https://huggingface.co/collections/minishlab/potion-6721e0abd4ea41881417f062):

| Model | Parameters | Notes |
|---|---|---|
| `minishlab/potion-base-2M` | 2M | Smallest, fastest |
| `minishlab/potion-base-8M` | 8M | Default — good balance |
| `minishlab/potion-base-32M` | 32M | Higher quality, slower |
| `minishlab/potion-retrieval-32M` | 32M | Optimized for retrieval tasks |
| `minishlab/potion-multilingual-128M` | 128M | 101 languages |

Any Model2Vec-compatible model on HuggingFace works — set the `name` to its model ID. You can pin a specific revision for reproducibility.

### Storage

Two Parquet files are written to `.mdvs/`:

- **`files.parquet`** — one row per file. Contains the filename, all frontmatter fields (in a single Struct column), a content hash, and a build timestamp.
- **`chunks.parquet`** — one row per chunk. Contains the chunk's position (file, index, line range) and its embedding vector.

The `files.parquet` holds your frontmatter as structured data — this is what `--where` filters query against. The `chunks.parquet` holds the vectors that similarity search operates on. The two are joined by file ID at query time.

## Incremental builds

Build only re-embeds what changed. Each file's markdown body (excluding frontmatter) is hashed, and the hash is compared against the existing index:

| Classification | Condition | Action |
|---|---|---|
| **New** | File not in index | Chunk, embed, add |
| **Edited** | Hash changed | Re-chunk, re-embed, replace chunks |
| **Unchanged** | Hash matches | Keep existing chunks |
| **Removed** | In index but not on disk | Drop file and its chunks |

Frontmatter-only changes (adding a tag, fixing a typo in `author`) update `files.parquet` without re-embedding — the body hash hasn't changed, so the vectors are still valid.

When nothing needs embedding, the model isn't even loaded. A `--force` flag triggers a full rebuild regardless of hashes.

## How search works

When you run `mdvs search "query" example_kb`:

1. The query text is embedded with the same model used during build
2. Every chunk's embedding is compared to the query via cosine similarity
3. For each file, only the **best chunk** score is kept — a file with one highly relevant section ranks above a file with uniformly mediocre content
4. Results are sorted by score (highest first) and limited by `--limit` (default 10)

This is brute-force search — every chunk is compared. For the typical vault size (hundreds to low thousands of files), this is fast enough. The entire search runs in-process with no external services.

### Scores

The score column in search output is cosine similarity — a value between 0 and 1, where higher means more similar. Scores depend on the model and the content, so there's no universal threshold for "relevant." Compare scores relative to each other within a single query.

## Filtering with `--where`

Add a SQL filter to narrow results by frontmatter fields:

```bash
mdvs search "calibration" example_kb --where "status = 'active'"
```

The `--where` clause filters on frontmatter fields — only files that match the filter are included in the results. The filter and similarity ranking are combined in a single query, so files that don't match are excluded efficiently.

You can use any SQL expression that DataFusion supports:

```bash
--where "draft = false"
--where "status = 'active' AND author = 'Giulia Ferretti'"
--where "sample_count > 10"
```

Array fields, nested objects, and field names with special characters require specific syntax — see the [Search Guide](../search-guide.md) for the full reference.

## Model identity

Search refuses to run if the model configured in `mdvs.toml` doesn't match the model that was used to build the index. This is a hard error, not a warning.

Embeddings from different models are incompatible — cosine similarity between vectors from different models produces meaningless scores. If you change the model, rebuild the index with `mdvs build --force`.
