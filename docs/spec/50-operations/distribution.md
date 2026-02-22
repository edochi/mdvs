# Distribution

**Status: DRAFT**

**Cross-references:** [Terminology](../01-terminology.md)

---

## Overview

Two binaries ship independently: `mdvs` (~20MB, full search) and `mfv` (~2MB, standalone validator). Both are single statically-linked Rust executables with no shared library dependencies.

---

## What Ships in the Binary

Everything compiled from Rust crates is baked in at build time:

| Component | Included via |
|---|---|
| DuckDB engine | `duckdb` crate with `bundled` feature |
| Model2Vec inference | `model2vec-rs` crate |
| Frontmatter parser | `gray_matter` crate |
| Markdown chunker | `text-splitter` crate |
| Markdown parser | `pulldown-cmark` crate |
| CLI framework | `clap` crate |

No shared libraries, no runtime interpreters, no system dependencies.

---

## What Downloads on First Run

`mdvs init` requires network access for two things:

| Download | Size | Cached At | Required For |
|---|---|---|---|
| DuckDB `vss` extension | ~few MB | `~/.duckdb/extensions/` | HNSW index, `array_cosine_distance()` |
| Embedding model weights | ~30MB (default) | `~/.cache/mdvs/models/` | Embedding inference |

After `init` completes, all subsequent operations are fully offline.

Progress bars (via `indicatif`) are shown during both downloads.

`mfv` has no first-run downloads — it works entirely offline from install.

---

## Distribution Channels

| Channel | Command | Installs |
|---|---|---|
| **crates.io** | `cargo install mdvs` | Full search tool (~20MB) |
| **crates.io** | `cargo install mfv` | Standalone validator (~2MB) |
| **GitHub Releases** | Download pre-built binary | Both binaries per release |
| **Homebrew tap** | `brew install <user>/tap/mdvs` | Full search tool |
| **Homebrew tap** | `brew install <user>/tap/mfv` | Standalone validator |

---

## Pre-Built Binaries

Primary distribution path. Built in CI via `cargo-dist` or cross-compilation.

### Target Platforms

| Target | OS | Arch |
|---|---|---|
| `x86_64-unknown-linux-gnu` | Linux | x86_64 |
| `aarch64-unknown-linux-gnu` | Linux | ARM64 |
| `x86_64-apple-darwin` | macOS | Intel |
| `aarch64-apple-darwin` | macOS | Apple Silicon |

Each release is a single compressed binary — download, extract, put in PATH.

---

## Dependency Comparison

| Tool | Install requires | Runtime requires |
|---|---|---|
| **mdvs** | Download one binary | First-run network for vss + model |
| **mfv** | Download one binary | Nothing |
| qmd | Node.js/Bun + npm | Ollama running |
| obsidian-note-taking-assistant | Python + pip/uv | Python runtime |
| mdrag | Rust toolchain or binary | Ollama running |

---

## Related Documents

- [Terminology](../01-terminology.md)
- [Crate: mdvs](../10-crates/mdvs/spec.md)
- [Crate: mfv](../10-crates/mfv/spec.md)
