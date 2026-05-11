# AGENTS.md

Guidance for AI coding agents working with this repository. Provider-agnostic — symlinked from `CLAUDE.md`, `.cursorrules`, and similar so any agent reads the same instructions.

## Project Overview

mdvs (Markdown Validation & Search) is a Rust CLI that treats markdown directories as databases — schema inference, frontmatter validation, and semantic search with SQL filtering. Single binary, no external services, instant embeddings via Model2Vec static models, DataFusion + Parquet for storage and search. Design specs live in `docs/spec/`, user-facing documentation in `book/`.

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
chore(deps): bump datafusion to 53
refactor: extract validate() from check command
```

## Build & Verify

```bash
cargo build                  # build
cargo run                    # run mdvs
cargo test                   # run all tests
cargo clippy --all-targets   # lint (including test code)
cargo fmt                    # format
```

**Always use `cargo clippy --all-targets`** — plain `cargo clippy` misses warnings in test code. **Always run `cargo fmt` after `cargo clippy`** when verifying changes.

## Architecture

Cargo workspace with two crates:

- **`crates/mdvs/`** — the CLI + library
- **`crates/tomljson/`** — lossless TOML↔JSON translation (used by schema loading)

mdvs modules, grouped by pipeline stage:

- **`src/discover/`** — `scan.rs` (walk + parse YAML), `field_type.rs` (FieldType enum + widening), `infer/{mod,types,paths,constraints}.rs` (DirectoryTree + GlobMap + InferredSchema, tracks observed_types per field)
- **`src/preprocess.rs`** — `ValueStage` enum (Stage 2 preprocessors), `Pipeline`, `infer_value_stages`
- **`src/schema/`** — `config.rs` (`MdvsToml`), `shared.rs` (common types), `json_schema.rs` (`dsl_to_canonical`, `canonical_to_dsl`, `validate_mdvs_schema`, `compute_schema_hash`), `load.rs` (extension-dispatched schema loader), `constraints/{categories,length,pattern,range}.rs`
- **`src/index/`** — `chunk.rs` (semantic chunking), `embed.rs` (model2vec embeddings), `storage.rs` (Parquet I/O, column constants, `BuildMetadata`)
- **`src/search.rs`** — cosine similarity UDF + DataFusion query, `SearchContext` with collision detection
- **`src/cmd/`** — `init`, `build`, `search`, `check`, `update`, `clean`, `info`, `export_jsonschema`

### Data Pipeline

`.md` files → frontmatter extraction (`gray_matter`) → schema translation (`dsl_to_canonical`) → Stage 2 preprocessors → per-field `jsonschema` validators → semantic chunking (`text-splitter` MarkdownSplitter) → plain text extraction (`pulldown-cmark`) → embeddings (`model2vec-rs`) → Parquet storage (`files.parquet` + `chunks.parquet`) → brute-force cosine similarity in Rust → DataFusion SQL for JOIN/aggregate/filter

### Key Design Decisions

- Two layers: validation (init/update/check — no model needed) and search (build/search — model + parquets)
- **Validation engine is `jsonschema`** (v0.46), not hand-rolled. `dsl_to_canonical(config)` translates mdvs.toml fields into a JSON Schema 2020-12 document; per-field `jsonschema::Validator` instances are compiled once per `validate()` call. Errors map exhaustively to `ViolationKind`.
- **Strict types.** `FieldType::String` is `{"type": "string"}` — no permissive set. Coercion is the preprocessor's job, declared per-field in `[[fields.field]].preprocess`.
- **Preprocessors are inference-driven.** `infer_value_stages` tracks observed_types per field and writes `coerce_to_string` / `widen_int_to_float` when widening was needed. `preprocess = []` means strict.
- Config-driven frontmatter fields: all frontmatter stored as native Arrow Struct column (`data`), no dynamic SQL columns. No interactive prompts.
- No lock file — `mdvs.toml` is the complete source of truth for validation. Build metadata + schema hash stored in parquet native key-value metadata.
- Build includes check internally — validates before embedding, aborts on violations
- Model identity tracking in parquet metadata: hard error on model/revision mismatch for both search and build
- **Schema hash** (xxh3 of canonical JSON of `dsl_to_canonical(config)`) detects field/type/constraint/preprocess changes between builds; mismatch requires `--force`.
- Note-level ranking uses max chunk similarity across chunks (not average)
- `--where` SQL clauses for metadata filtering (no custom filter syntax)
- `--output` global flag (`text`/`json`) via `CommandOutput` trait
- All text processing and vector math in Rust; DataFusion handles SQL query execution
- **Enum-based dispatch everywhere** (no `dyn Trait`): `FieldType`, `Backend`, `Embedder`, `ConstraintKind`, `ValueStage`, `Outcome`. Exhaustive matches.
- Config validation on load: **five invariants** (ignore/field mutual exclusion, valid glob format, required ⊆ allowed, constraints valid for type, preprocess applicability + no duplicates)

### Storage

- Two artifacts: `mdvs.toml` (committed) + `.mdvs/` (gitignored)
- `files.parquet`: `file_id`, `filepath`, `data` (frontmatter Struct), `content_hash`, `built_at`
- `chunks.parquet`: `chunk_id`, `file_id` FK, `chunk_index`, `start_line`, `end_line`, `embedding` FixedSizeList<Float32>
- Column names are fixed constants (`COL_FILE_ID`, `COL_FILEPATH`, etc.) — no prefix in storage
- Build metadata (model, revision, chunk_size, glob, built_at, **schema_hash**) stored as parquet native key-value metadata
- Internal column prefix/aliases applied at search view layer only (`[search].internal_prefix`, `[search.aliases]`)

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
- `build [path]` — check + embed + write Parquets to `.mdvs/`
- `search <query> [path]` — query the index
- `info [path]` — show config and index status
- `clean [path]` — delete `.mdvs/`
- `export-jsonschema [path]` — translate `[fields]` into a JSON Schema 2020-12 document (`--format json|toml`, `--output-file FILE`)

See `docs/spec/commands/` for detailed specs and `book/src/commands/` for user-facing docs.

## Key Dependencies

| Crate | Purpose |
|---|---|
| `datafusion` | SQL query engine on Arrow arrays |
| `parquet` / `arrow` | Columnar storage and in-memory format |
| `jsonschema` | JSON Schema 2020-12 per-value validator (Wave B engine) |
| `tomljson` | Workspace crate: lossless TOML↔JSON for `.toml` schema files |
| `model2vec-rs` | Static embedding inference (POTION models, no GPU) |
| `gray_matter` | YAML frontmatter extraction |
| `text-splitter` (markdown) | Semantic chunking |
| `pulldown-cmark` | Markdown → plain text |
| `clap` | CLI parsing |
| `tokio` | Async runtime (required by DataFusion) |
| `tabled` | Table rendering |
| `globset` | Glob pattern matching for allowed/required validation |
| `regex` | Pattern constraint compilation |
| `xxhash-rust` | Content hash + schema hash (xxh3-64) |
| `cocogitto` | Conventional commit enforcement (dev tool, not a dependency) |
