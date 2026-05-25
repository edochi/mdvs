# Search Modes

`mdvs search` runs in one of three modes, controlled by `--mode`:

```bash
mdvs search "<query>" [path] --mode {semantic|fulltext|hybrid}
```

The default is `hybrid`. The right mode depends on what kind of question you're asking and how confident you are about the wording.

## TL;DR — which mode when

| You want to find… | Pick |
|---|---|
| Something whose wording you can paraphrase but not quote | `semantic` |
| An exact identifier, acronym, error message, or filename | `fulltext` |
| Anything — let mdvs combine both signals | `hybrid` (default) |

If you're not sure, leave it on `hybrid`. It tends to do at least as well as either alone, at the cost of one extra index lookup that's effectively free.

## What each mode actually does

### `semantic` — meaning, not words

The query is embedded into a vector with the same Model2Vec model used to build the index, and chunks are ranked by cosine similarity to that vector. Two chunks score similarly when they're *about* similar things, even if they share no words.

This is the mode that does the magic:

```bash
mdvs search "how to get in touch" --mode semantic
# matches a chunk that says "reach out via Slack" with no shared words
```

It's also the mode that has nothing to fall back on when your query is an acronym or a unique string that the model doesn't have a meaningful embedding for.

### `fulltext` — words, not meaning

The query is tokenized and scored against the BM25 inverted index on the persisted `chunk_text` column. No embedding model is loaded; this mode also works when no model has been downloaded yet.

Use it when you know the exact term you're after:

```bash
mdvs search "SPR-A1" --mode fulltext           # exact equipment ID
mdvs search "calibration.toml" --mode fulltext # exact filename
mdvs search "TODO-0159" --mode fulltext        # exact ticket reference
```

BM25 doesn't care about meaning at all. A search for `"how to get in touch"` in fulltext mode will only match chunks that contain some of those exact words.

### `hybrid` — both, reranked

Hybrid runs both `semantic` and `fulltext` queries, then merges the two ranked lists with LanceDB's [Reciprocal Rank Fusion](https://en.wikipedia.org/wiki/Reciprocal_rank_fusion) reranker. The result is a single ranking that promotes documents which scored well on **either** signal.

In practice this means:

- A natural-language query that has no exact lexical matches still ranks the semantically-closest chunks at the top.
- An exact-identifier query still surfaces the chunk that contains it verbatim, even if its surrounding context is semantically unremarkable.
- Queries that are *both* — a phrase that mixes a concept with a specific term — get the best of both rankings.

Hybrid is the default because it makes the system tolerate vague queries and precise queries with the same flag.

## Scores aren't comparable across modes

The `score` column in the output means something different in each mode:

| Mode | Score |
|---|---|
| `semantic` | Cosine similarity. Roughly `[0, 1]`. Higher = more similar in meaning. |
| `fulltext` | BM25 relevance score. Unbounded; depends on corpus size and term rarity. Higher = better lexical match. |
| `hybrid` | RRF relevance score. Unbounded but small. Higher = better. |

Don't compare scores across runs that used different modes. Within a single run, the *ordering* of the hits is what matters.

## Performance and indexing

- **`semantic`** needs the embedding model loaded. On the first run that's a one-time ~30 MB download (default model). Subsequent runs reuse the cached model.
- **`fulltext`** doesn't need the model at all and works as soon as `mdvs build` has been run.
- **`hybrid`** does the semantic + fulltext work in parallel; the only extra cost over `semantic` alone is the BM25 lookup, which is negligible at most vault sizes.

All three modes use the same Lance dataset under `.mdvs/`. The BM25 full-text index on `chunk_text` is built every time `mdvs build` runs; the cosine IVF-PQ vector index on `embedding` is built only when the index exceeds 10,000 chunks (smaller vaults rely on LanceDB's exact flat scan, which is plenty fast at that scale). See [Search & Indexing](./search.md#storage) for the storage layout.

## Combining with `--where`

Mode is independent of `--where`. Any mode can be paired with any SQL filter:

```bash
mdvs search "drift" --mode fulltext --where "status = 'active'"
mdvs search "how the project ended" --mode semantic --where "joined < '2025-01-01'"
mdvs search "calibration" --where "draft = false"          # default mode is hybrid
```

The filter narrows which chunks LanceDB considers; the mode decides how they're ranked within that narrowed set. See the [Search Guide](../search-guide.md) for the full `--where` reference.
