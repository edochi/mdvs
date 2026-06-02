# mdvs vs QMD — benchmark report

_Generated 2026-06-02 17:20_
_Corpora: `example_kb`, `docs`_
_mdvs 0.6.2 · QMD 2.5.2_

This report characterises how mdvs and QMD compare on warm/steady-state search latency, peak memory, build time, and output footprint. See [TODO-0166](../spec/todos/TODO-0166.md) for the framing and decisions behind what's measured (and what's deliberately not).

## Methodology

For each (tool, corpus, query) combination the runner records:

- **Warm/steady-state search latency** — `/usr/bin/time -l` wall time, median of N iterations after one warm-up invocation
- **Peak resident set size** — maximum RSS observed during the query
- **CPU%** — derived from `(user + sys) / wall × 100`; indicates whether wall time was CPU-bound or I/O-bound
- **Index build time** — single timed run from clean state (`rm -rf .mdvs && mdvs build --force` for mdvs; `qmd collection add` + `qmd embed -f` for QMD)
- **Index size on disk** — `du -sk` after build
- **Output token count** — `tiktoken` cl100k_base over the result snippets, for `--limit 10`
- **Tool footprint on disk** — binary + cached embedding/reranker models

Cold-start latency and page-fault counts are deliberately excluded; see [TODO-0166](../spec/todos/TODO-0166.md) for the rationale.

Each search runs three iterations preceded by one warm-up invocation, so the reported wall time and RSS reflect steady-state behaviour.

## Reading the numbers fairly

The two tools have meaningfully different feature sets. Any conclusions drawn from these numbers should respect the following:

- **QMD `query` runs LLM reranking and query expansion** on top of BM25 + vector. mdvs's hybrid mode uses RRF only — no LLM in the loop. The reranking step changes both latency and quality; comparing wall times alone understates QMD's quality work
- **QMD does AST-aware chunking** for source code (TypeScript, JavaScript, Python, Go, Rust); mdvs uses prose chunking via `text-splitter`'s `MarkdownSplitter`. On code-heavy corpora the chunking strategies will produce different recall/precision profiles, independent of search engine speed
- **mdvs has `--where` SQL filtering and frontmatter validation**; QMD has neither. These are feature presence, not performance, and don't appear in the metric tables
- **Embedding models differ in size and quality.** mdvs uses Model2Vec `potion-base-8M` (~30 MB static distillation); QMD uses `embeddinggemma-300M-Q8_0` (~300 MB GGUF). Smaller model → less memory and faster load, but a different quality ceiling
- **Default chunking and limits differ.** mdvs uses 1024-char chunks; QMD's default chunking produces roughly one chunk per file on this corpus. Result count and token count comparisons should be read with this in mind

This benchmark measures **latency, footprint, and setup cost** under each tool's defaults. It does not measure ranking quality — that would require a labelled query set and is out of scope.

## Test environment

| | |
|---|---|
| OS | `macOS-26.5-arm64-arm-64bit` |
| CPU arch | `arm64` |
| Python | `3.11.14` |
| mdvs version | `mdvs 0.6.2` |
| qmd version | `qmd 2.5.2` |
| Iterations per query | 3 (+ 1 warm-up) |
| --limit | 10 |

## Corpus: `example_kb` (46 files)

### Setup (full from-scratch build, both phases timed)

Both tools are set up fresh each run. The two phases are timed separately:

- **prepare** — mdvs `init` (schema inference) / QMD `collection add` (scan + chunk + metadata)
- **index** — mdvs `build --force` (scan + chunk + validate + embed) / QMD `embed -f` (vectors)

(mdvs bundles scan/chunk/validate into `build`; QMD splits them into `collection add`. The **total** is the comparable figure — raw files to a queryable index.)

| | mdvs | QMD |
|---|---|---|
| Prepare (init / collection add) | 190 ms | 180 ms |
| Index (build / embed) | 260 ms | 3.55 s |
| **Total setup** | 450 ms | 3.73 s |
| Index peak RSS | 125 MB | 816 MB |
| Index on disk | 232.0 KB | 69.7 MB |
| Embedding/reranker models on disk | 59.0 MB | 2.10 GB |

### Queries

| Kind | Query | mdvs mode | `--where` clause |
|---|---|---|---|
| `broad_semantic` | _"calibration baseline"_ | `semantic` | — |
| `narrow_semantic` | _"wavelet denoising replication"_ | `semantic` | — |
| `exact_phrase` | _"SPR-A1"_ | `fulltext` | — |
| `metadata_filtered` | _"calibration"_ | `hybrid` | `status = 'completed'` |
| `vague_multiword` | _"what went wrong with the spectrometer"_ | `hybrid` | — |

### Search latency (warm, median of N)

mdvs is reported in two configurations:

- **mdvs default** — runs as users typically invoke it; `auto_update` and `auto_build` in `mdvs.toml` cause a scan + frontmatter-validation + build-check pass before every search (~110 ms on this corpus)
- **mdvs engine-only** — same query with `--no-update --no-build`. Measures the search engine itself without the orchestration overhead. Closer to a like-for-like comparison with QMD, which has no equivalent feature

| Kind | mdvs default | mdvs engine-only | mdvs RSS | mdvs CPU% | QMD mode | QMD wall | QMD RSS | QMD CPU% |
|---|---|---|---|---|---|---|---|---|
| `broad_semantic` | 230 ms | 210 ms | 130 MB | 26% | `vsearch` | 800 ms | 613 MB | 86% |
| `narrow_semantic` | 230 ms | 210 ms | 130 MB | 30% | `vsearch` | 790 ms | 626 MB | 87% |
| `exact_phrase` | 210 ms | 190 ms | 53.3 MB | 20% | `search` | 160 ms | 66.7 MB | 94% |
| `metadata_filtered` | 230 ms | 210 ms | 134 MB | 30% | — | — | — | — |
| `vague_multiword` | 230 ms | 210 ms | 132 MB | 30% | `query` | 700 ms | 638 MB | 107% |

### Output token count (snippets for `--limit 10`, `tiktoken` `cl100k_base`)

Token count matters when results are piped into a downstream LLM — fewer tokens = less context spent.

| Kind | mdvs result count | mdvs tokens | QMD result count | QMD tokens |
|---|---|---|---|---|
| `broad_semantic` | 10 | 1,712 | 10 | 444 |
| `narrow_semantic` | 10 | 1,373 | 10 | 566 |
| `exact_phrase` | 10 | 1,601 | 7 | 376 |
| `metadata_filtered` | 5 | 974 | — | — |
| `vague_multiword` | 10 | 1,518 | 10 | 605 |

### Notes

- _qmd_: QMD uses a global ~/.cache/qmd/index.sqlite; index_size_bytes includes any unrelated user collections
- _qmd_: skipped 'metadata_filtered': qmd has no --where equivalent

## Corpus: `docs` (1669 files)

### Setup (full from-scratch build, both phases timed)

Both tools are set up fresh each run. The two phases are timed separately:

- **prepare** — mdvs `init` (schema inference) / QMD `collection add` (scan + chunk + metadata)
- **index** — mdvs `build --force` (scan + chunk + validate + embed) / QMD `embed -f` (vectors)

(mdvs bundles scan/chunk/validate into `build`; QMD splits them into `collection add`. The **total** is the comparable figure — raw files to a queryable index.)

| | mdvs | QMD |
|---|---|---|
| Prepare (init / collection add) | 460 ms | 1.70 s |
| Index (build / embed) | 2.72 s | 843.94 s |
| **Total setup** | 3.18 s | 845.64 s |
| Index peak RSS | 352 MB | 975 MB |
| Index on disk | 31.6 MB | 69.7 MB |
| Embedding/reranker models on disk | 59.0 MB | 2.10 GB |

### Queries

| Kind | Query | mdvs mode | `--where` clause |
|---|---|---|---|
| `broad_semantic` | _"deploying applications to kubernetes"_ | `semantic` | — |
| `narrow_semantic` | _"rolling update strategy"_ | `semantic` | — |
| `exact_phrase` | _"kubectl apply"_ | `fulltext` | — |
| `metadata_filtered` | _"minikube"_ | `hybrid` | `content_type = 'tutorial'` |
| `vague_multiword` | _"how do I expose my service to the internet"_ | `hybrid` | — |

### Search latency (warm, median of N)

mdvs is reported in two configurations:

- **mdvs default** — runs as users typically invoke it; `auto_update` and `auto_build` in `mdvs.toml` cause a scan + frontmatter-validation + build-check pass before every search (~110 ms on this corpus)
- **mdvs engine-only** — same query with `--no-update --no-build`. Measures the search engine itself without the orchestration overhead. Closer to a like-for-like comparison with QMD, which has no equivalent feature

| Kind | mdvs default | mdvs engine-only | mdvs RSS | mdvs CPU% | QMD mode | QMD wall | QMD RSS | QMD CPU% |
|---|---|---|---|---|---|---|---|---|
| `broad_semantic` | 1.73 s | 330 ms | 383 MB | 605% | `vsearch` | 810 ms | 621 MB | 95% |
| `narrow_semantic` | 1.82 s | 330 ms | 384 MB | 598% | `vsearch` | 800 ms | 630 MB | 92% |
| `exact_phrase` | 1.82 s | 310 ms | 384 MB | 622% | `search` | 160 ms | 72.1 MB | 94% |
| `metadata_filtered` | 1.84 s | 320 ms | 387 MB | 610% | — | — | — | — |
| `vague_multiword` | 1.91 s | 340 ms | 388 MB | 619% | `query` | 770 ms | 649 MB | 104% |

### Output token count (snippets for `--limit 10`, `tiktoken` `cl100k_base`)

Token count matters when results are piped into a downstream LLM — fewer tokens = less context spent.

| Kind | mdvs result count | mdvs tokens | QMD result count | QMD tokens |
|---|---|---|---|---|
| `broad_semantic` | 10 | 459 | 10 | 607 |
| `narrow_semantic` | 10 | 554 | 10 | 604 |
| `exact_phrase` | 10 | 869 | 10 | 440 |
| `metadata_filtered` | 10 | 1,031 | — | — |
| `vague_multiword` | 10 | 1,608 | 10 | 555 |

### Notes

- _qmd_: QMD uses a global ~/.cache/qmd/index.sqlite; index_size_bytes includes any unrelated user collections
- _qmd_: skipped 'metadata_filtered': qmd has no --where equivalent
