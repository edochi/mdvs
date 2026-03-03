# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## 0.1.0

Initial release.

### Added

- 7 commands: `init`, `check`, `update`, `build`, `search`, `info`, `clean`
- Semantic search over markdown directories using Model2Vec static embeddings
- Frontmatter validation with configurable schema (`mdvs.toml`)
- DataFusion + Parquet storage (no external database)
- Incremental builds — only re-embeds files whose content changed
- `--where` SQL filtering on frontmatter fields
- Human and JSON output modes (`--output`)
- `--verbose` flag with tracing instrumentation
