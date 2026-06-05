# AGENTS.md

Guidance for AI coding agents working with this repository. Provider-agnostic — symlinked from `CLAUDE.md`, `.cursorrules`, and similar so any agent reads the same instructions.

## Project Overview

mdvs (Markdown Validation & Search) is a Rust CLI that treats markdown directories as databases — schema inference, frontmatter validation, and semantic/full-text/hybrid search with SQL filtering. Single binary, no external services, instant embeddings via Model2Vec static models, [LanceDB](https://lancedb.com/) for storage + native search (cosine vector + BM25 FTS + RRF hybrid). Design specs live in `docs/spec/`, user-facing documentation in `book/`.

## Git Rules

**Never push directly to `main`.** All work goes through feature branches and PRs. One branch per TODO or feature (`feat/description`, `fix/description`, `docs/description`). Regular merge (not squash). Always ask the user before creating a branch.

**Releases** go through a `release/v<version>` branch + PR, then a tag push on main triggers the build.

**NEVER commit or push unless the user explicitly asks.** No autonomous commits. No "let me commit this" — wait for the user to say "commit" or "commit and push". This is non-negotiable.

**Use conventional commits.** All commit messages must follow the [Conventional Commits](https://www.conventionalcommits.org/) format. A `commit-msg` hook (via cocogitto) enforces this locally. See `docs/spec/cocogitto.md` for the full guide.

Format: `<type>[optional scope]: <description>`

Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`

Examples:
```
feat: add enum constraints on string fields
fix(build): track removed chunk counts correctly
docs: add cocogitto setup guide
chore(deps): bump lancedb to 0.29
refactor: extract validate() from check command
```

## Build & Verify

```bash
cargo build                                            # build (production — no mock)
cargo run                                              # run mdvs
cargo test                                             # run fast-lane tests (uses MockEmbedder under cfg(test))
cargo test --features testing-mocks                    # explicit fast lane (what CI runs)
cargo test --features testing-mocks -- --ignored       # slow lane: real-model tests (local only, needs HF cache)
cargo clippy --all-targets --features testing-mocks    # lint, mirroring CI flags
cargo fmt                                              # format
```

**Always use `cargo clippy --all-targets --features testing-mocks`** — plain `cargo clippy` misses warnings in test code and the mock feature gate; this matches the CI invocation. **Always run `cargo fmt` after `cargo clippy`** when verifying changes.

The `testing-mocks` feature gates the deterministic `MockEmbedder` (`provider = "mock"` in `mdvs.toml`). It is off in production binaries (`cargo install`); `cargo test` and `cargo clippy` see it via `cfg(test)`. Real-model tests are marked `#[ignore]` so the fast lane stays hermetic — no Hugging Face network calls. See TODO-0184.

## Architecture

Cargo workspace with two crates:

- **`crates/mdvs/`** — the CLI + library
- **`crates/tomljson/`** — lossless TOML↔JSON translation (used by schema loading)

mdvs modules, grouped by pipeline stage:

- **`src/discover/`** — `scan.rs` (walk + per-file frontmatter dispatch across YAML / TOML / JSON), `field_type.rs` (FieldType enum + widening), `infer/{mod,types,paths,constraints}.rs` (DirectoryTree + GlobMap + InferredSchema, tracks observed_types per field)
- **`src/preprocess.rs`** — `ValueStage` enum (Stage 2 preprocessors), `Pipeline`, `infer_value_stages`
- **`src/schema/`** — `config.rs` (`MdvsToml`), `shared.rs` (common types), `json_schema.rs` (`dsl_to_canonical`, `canonical_to_dsl`, `validate_mdvs_schema`, `compute_schema_hash`), `load.rs` (extension-dispatched schema loader), `constraints/{categories,length,pattern,range}.rs`
- **`src/index/`** — `chunk.rs` (semantic chunking), `embed.rs` (model2vec embeddings), `storage.rs` (Arrow batch construction, column constants, `BuildMetadata`), `backend.rs` (`LanceBackend`: LanceDB connection, `write_index`, `search` with mode dispatch, `--where` translator)
- **`src/search.rs`** — `SearchMode` enum (`Semantic` / `Fulltext` / `Hybrid`, default `Hybrid`), per-mode score column resolution, collision detection
- **`src/cmd/`** — `init`, `build`, `search`, `check`, `update`, `clean`, `info`, `export_jsonschema`

### Data Pipeline

`.md` files → per-file frontmatter format detection (`---` YAML, `+++` TOML, `{` JSON; `[scan].frontmatter_format` can force one) → frontmatter extraction (YAML + TOML via `gray_matter`; JSON via `serde_json::Deserializer::byte_offset`) → schema translation (`dsl_to_canonical`) → Stage 2 preprocessors → per-field `jsonschema` validators → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → Lance storage (single `.mdvs/index.lance/` dataset, one row per chunk) → LanceDB native search (cosine `nearest_to` / BM25 `full_text_search` / RRF hybrid) + LanceDB SQL filter for `--where`

### Key Design Decisions

- Two layers: validation (init/update/check — no model needed) and search (build/search — model + Lance index)
- **Validation engine is `jsonschema`** (v0.46), not hand-rolled. `dsl_to_canonical(config)` translates mdvs.toml fields into a JSON Schema 2020-12 document; per-field `jsonschema::Validator` instances are compiled once per `validate()` call. Errors map exhaustively to `ViolationKind`.
- **Strict types.** `FieldType::String` is `{"type": "string"}` — no permissive set. Coercion is the preprocessor's job, declared per-field in `[[fields.field]].preprocess`.
- **Preprocessors are inference-driven.** `infer_value_stages` tracks observed_types per field and writes `coerce_to_string` / `widen_int_to_float` when widening was needed. `preprocess = []` means strict.
- Config-driven frontmatter fields: all frontmatter stored as native Arrow Struct column (`data`), no dynamic SQL columns. No interactive prompts.
- No lock file — `mdvs.toml` is the complete source of truth for validation. Build metadata + schema hash stored as Lance table-level key-value metadata.
- Build includes check internally — validates before embedding, aborts on violations
- Model identity tracking in Lance table metadata: hard error on model/revision mismatch for both search and build
- **Schema hash** (xxh3 of canonical JSON of `dsl_to_canonical(config)`) detects field/type/constraint/preprocess changes between builds; mismatch requires `--force`.
- Note-level ranking uses max chunk similarity across chunks (not average); best-chunk-per-file dedupe runs in Rust after LanceDB returns ranked rows (`OVER_FETCH_FACTOR=3` to compensate)
- **Search modes** — `--mode {semantic,fulltext,hybrid}`; default `hybrid`. Semantic = cosine `nearest_to`; fulltext = BM25 `full_text_search` on persisted `chunk_text`; hybrid = LanceDB's RRF reranker over both. FTS index built always; cosine IVF-PQ vector index built only above `VECTOR_INDEX_MIN_ROWS=10_000` (smaller vaults use exact flat scan)
- `--where` SQL clauses for metadata filtering — translated to LanceDB's SQL filter; the translator prefixes bare frontmatter names with `data.` and rejects `Array(Float)` fields (a Lance encoding panic; see TODO-0159)
- `--output` global flag (`text`/`json`) via `CommandOutput` trait
- All text processing in Rust; LanceDB executes the search query (vector ANN / FTS / hybrid + filter)
- **Enum-based dispatch everywhere** (no `dyn Trait`): `FieldType`, `Backend`, `Embedder`, `ConstraintKind`, `ValueStage`, `SearchMode`, `Outcome`. Exhaustive matches.
- Config validation on load: **eight invariants** (ignore/field mutual exclusion, valid glob format, required ⊆ allowed, constraints valid for type, preprocess applicability + no duplicates, no top-level Object, dotted-name well-formedness, no leaf-vs-parent shape conflicts)
- **Dotted-name leaf flattening (Wave C, TODO-0097)**: `[[fields.field]]` names may contain `.` to express nested frontmatter structure (`calibration.baseline.wavelength`) regardless of source format (YAML mapping, TOML table, JSON object). Top-level Object is rejected at config load; `Array(Object{...})` stays inline. Translator (`dsl_to_canonical`) reconstructs the canonical JSON Schema's nested `properties` tree; `canonical_to_dsl` reverses. Storage transposes the flat toml into a synthetic FieldType::Object before building Arrow Structs, so the `data` column matches the source frontmatter's natural nesting and SQL dot-notation `--where` works natively.
- **Multi-format frontmatter (TODO-0162)**: per-file auto-detect from the leading delimiter (`---` YAML, `+++` TOML, `{` JSON). YAML + TOML go through `gray_matter` with engine-specific delimiters; JSON bypasses `gray_matter` and uses `serde_json::Deserializer::into_iter().byte_offset()` to handle the Hugo-style bare-braces convention. Configurable via `[scan].frontmatter_format` (`"auto"` default; `"yaml"`/`"toml"`/`"json"` force one and surface a `FrontmatterUnrepresentable` violation on mismatch).

### Storage

- Two artifacts: `mdvs.toml` (committed) + `.mdvs/` (gitignored)
- Single Lance dataset at `.mdvs/index.lance/` (table name `index`), **one row per chunk**:
  `chunk_id`, `file_id`, `chunk_index`, `start_line`, `end_line`, `chunk_text` (Utf8, persisted for FTS + snippets), `embedding` FixedSizeList<Float32, dim>, `filepath`, `content_hash`, `data` (frontmatter Struct), `built_at`
- Column names are fixed constants (`COL_FILE_ID`, `COL_FILEPATH`, `COL_CHUNK_TEXT`, etc.) — no prefix in storage
- Build metadata (model, revision, chunk_size, glob, built_at, **schema_hash**) stored as Lance table-level key-value metadata
- Indexes inside the dataset: FTS (inverted) on `chunk_text` always; cosine IVF-PQ on `embedding` only above `VECTOR_INDEX_MIN_ROWS=10_000`
- Internal column prefix/aliases applied at the `--where` translator level only (`[search].internal_prefix`, `[search.aliases]`)

### Configuration

`mdvs.toml` sections: `[scan]`, `[embedding_model]`, `[chunking]`, `[update]`, `[search]`, `[fields]` + `[[fields.field]]`

- Validation sections (`[scan]`, `[update]`, `[fields]`): always present
- Build sections (`[embedding_model]`, `[chunking]`, `[search]`): added by `init --auto-build` or by `build`
- `[search]` also holds `internal_prefix` and `aliases` for column naming in `--where` queries
- `[[fields.field]]` carries `name`, `type`, `allowed`, `required`, `nullable`, `constraints`, **`preprocess`**

### Commands

- `init [path]` — scan, infer schema, write `mdvs.toml`. `--from-jsonschema PATH` imports an external JSON Schema file instead.
- `check [path]` — validate frontmatter against schema (read-only). `--jsonschema PATH` overrides the `[fields]` block for this run.
- `update [path]` — re-scan, infer new fields, update `mdvs.toml`
- `build [path]` — check + embed + write the Lance dataset to `.mdvs/`
- `search <query> [path]` — query the index (`--mode {semantic,fulltext,hybrid}`, default `hybrid`)
- `info [path]` — show config and index status
- `clean [path]` — delete `.mdvs/`
- `export-jsonschema [path]` — translate `[fields]` into a JSON Schema 2020-12 document (`--format json|toml`, `--output-file FILE`)

See `docs/spec/commands/` for detailed specs and `book/src/commands/` for user-facing docs.

## Key Dependencies

| Crate | Purpose |
|---|---|
| `lancedb` / `lance-index` | Columnar storage + vector / FTS / hybrid search (v0.29 / =6.0.0) |
| `arrow` | In-memory columnar format (batches handed to LanceDB) |
| `jsonschema` | JSON Schema 2020-12 per-value validator (Wave B engine) |
| `tomljson` | Workspace crate: lossless TOML↔JSON for `.toml` schema files |
| `model2vec-rs` | Static embedding inference (POTION models, no GPU) |
| `gray_matter` | YAML + TOML frontmatter extraction (JSON parsed natively via `serde_json::Deserializer::byte_offset`) |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |
| `tokio` | Async runtime (required by LanceDB) |
| `futures` | Stream consumption from LanceDB query results |
| `tabled` | Table rendering |
| `globset` | Glob pattern matching for allowed/required validation |
| `regex` | Pattern constraint compilation |
| `xxhash-rust` | Content hash + schema hash (xxh3-64) |
| `cocogitto` | Conventional commit enforcement (dev tool, not a dependency) |
