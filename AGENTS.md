# AGENTS.md

Guidance for AI coding agents working with this repository. Provider-agnostic — symlinked from `CLAUDE.md`, `.cursorrules`, and similar so any agent reads the same instructions.

## Project Overview

mdvs (Markdown Validation & Search) is a Rust CLI that treats markdown directories as databases — schema inference, frontmatter validation, and semantic/full-text/hybrid search with SQL filtering. Single binary, no external services. Design specs live in `docs/spec/`, user-facing docs in `book/`.

## Git Rules

**Never push directly to `main`.** All work goes through feature branches and PRs. One branch per TODO or feature (`feat/description`, `fix/description`, `docs/description`). Regular merge (not squash). Always ask the user before creating a branch.

**Releases** go through a `release/v<version>` branch + PR, then a tag push on main triggers the build.

**NEVER commit or push unless the user explicitly asks.** No autonomous commits. No "let me commit this" — wait for the user to say "commit" or "commit and push". This is non-negotiable.

**Use conventional commits.** A `commit-msg` hook (cocogitto) enforces the format `<type>[optional scope]: <description>` locally. Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `perf`, `style`. See `docs/spec/cocogitto.md` for the full guide.

## Build & Verify

```bash
cargo build                                            # build (production — no mock)
cargo run                                              # run mdvs
cargo test                                             # fast lane (MockEmbedder via cfg(test))
cargo test --features testing-mocks                    # explicit fast lane (what CI runs)
cargo test --features testing-mocks -- --ignored       # slow lane: real-model tests (local, needs HF cache)
cargo clippy --all-targets --features testing-mocks    # lint (matches CI)
cargo fmt                                              # format
```

**Always use `cargo clippy --all-targets --features testing-mocks`** — plain `cargo clippy` misses warnings in test code and the mock feature gate. **Run `cargo fmt` after `cargo clippy`.**

The `testing-mocks` feature gates the deterministic `MockEmbedder` (`provider = "mock"` in `mdvs.toml`). It is off in production binaries (`cargo install`); `cargo test` and `cargo clippy` see it via `cfg(test)`. Real-model tests are marked `#[ignore]` so the fast lane stays hermetic — no Hugging Face network calls. See TODO-0184.

## Architectural Invariants

These survive across refactors; reach for them when in doubt:

- **Enum dispatch, no `dyn Trait`.** Backends, embedders, value stages, constraint kinds, search modes, outcomes are all enums with exhaustive matches. Adding a variant must update every match.
- **Two layers.** Validation (`init` / `update` / `check`) needs no embedding model. Search (`build` / `search`) needs the model + the Lance index in `.mdvs/`. Validation must stand alone.
- **Strict types.** `FieldType::String` rejects bools and numbers. Coercion is the preprocessor pipeline's job (`[[fields.field]].preprocess`), not the schema's.
- **Single source of truth.** `mdvs.toml` is the schema (committed); `.mdvs/` is build state (gitignored, recreatable). No lock file.
- **Build includes check.** Validation runs before embedding; violations abort the build.
- **No interactive prompts.** Every flow is config-driven until 1.0.

## Pointers

- Architecture, data pipeline, storage layout, design decisions: `docs/spec/architecture.md`, `docs/spec/storage.md`, per-command pages under `docs/spec/commands/`.
- Commands and flags: `mdvs --help` and `docs/spec/commands/`.
- Dependencies and their roles: comments in `crates/mdvs/Cargo.toml`.
- Skills (agent workflows): `.claude/skills/` — invoke via `Skill` for `commit`, `todo`, `rust`, `spec`, `code-editing`, etc.
- TODOs (in-flight + done): `docs/spec/todos/index.md`.
