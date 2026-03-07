# TODOs

| ID | Title | Status | Priority | Created |
|----|-------|--------|----------|---------|
| [0001](TODO-0001.md) | Null-transparent type widening | done | high | 2026-03-02 |
| [0002](TODO-0002.md) | Add .mdvsignore and .gitignore support | done | high | 2026-03-02 |
| [0003](TODO-0003.md) | Fix --auto-build flag on init | done | medium | 2026-03-02 |
| [0004](TODO-0004.md) | Rename --where-clause to --where | done | low | 2026-03-02 |
| [0005](TODO-0005.md) | Differentiate null from absent in check | todo | low | 2026-03-02 |
| [0006](TODO-0006.md) | Support enum constraints on string fields | todo | medium | 2026-03-02 |
| [0007](TODO-0007.md) | Support Date field type | todo | medium | 2026-03-02 |
| [0008](TODO-0008.md) | Support value boundary constraints on numeric fields | todo | low | 2026-03-02 |
| [0009](TODO-0009.md) | Custom field and text processors | todo | low | 2026-03-02 |
| [0010](TODO-0010.md) | Support length constraints on strings and arrays | todo | low | 2026-03-02 |
| [0011](TODO-0011.md) | Incremental build | done | medium | 2026-03-02 |
| [0012](TODO-0012.md) | Store build metadata in parquet | done | high | 2026-03-02 |
| [0013](TODO-0013.md) | Search verifies model against parquet metadata | done | high | 2026-03-02 |
| [0014](TODO-0014.md) | Build detects manual config changes via parquet metadata | done | high | 2026-03-02 |
| [0015](TODO-0015.md) | Implement info and clean commands | done | high | 2026-03-02 |
| [0016](TODO-0016.md) | LanceDB backend via compile-time feature flag | todo | medium | 2026-03-02 |
| [0017](TODO-0017.md) | Ollama embedding provider | todo | medium | 2026-03-02 |
| [0018](TODO-0018.md) | Cloud embedding providers (Azure, AWS Bedrock) | todo | low | 2026-03-02 |
| [0019](TODO-0019.md) | Global --verbose flag | done | medium | 2026-03-02 |
| [0020](TODO-0020.md) | Add required Cargo.toml metadata for crates.io | done | high | 2026-03-03 |
| [0021](TODO-0021.md) | Downgrade edition from 2024 to 2021 | done | high | 2026-03-03 |
| [0022](TODO-0022.md) | Update README for release | done | high | 2026-03-03 |
| [0023](TODO-0023.md) | Clean up stale .gitignore entries | done | low | 2026-03-03 |
| [0024](TODO-0024.md) | Add CHANGELOG.md | done | medium | 2026-03-03 |
| [0025](TODO-0025.md) | Trim published package size | done | low | 2026-03-03 |
| [0026](TODO-0026.md) | Fix clippy collapsible_if warnings | done | low | 2026-03-03 |
| [0027](TODO-0027.md) | Prefix internal parquet columns to avoid frontmatter collisions | done | medium | 2026-03-03 |
| [0028](TODO-0028.md) | Bare field names in --where clauses | done | medium | 2026-03-03 |
| [0029](TODO-0029.md) | User documentation site with mdBook | todo | medium | 2026-03-03 |
| [0030](TODO-0030.md) | Homebrew tap and prebuilt binaries | superseded | medium | 2026-03-03 |
| [0031](TODO-0031.md) | Example vault repository | todo | medium | 2026-03-04 |
| [0032](TODO-0032.md) | Fix verbose tracing output — show timing and useful info | done | high | 2026-03-04 |
| [0033](TODO-0033.md) | Unified output format (umbrella) | done | high | 2026-03-04 |
| [0034](TODO-0034.md) | Flag rework — rename human→text, add --logs, repurpose -v | done | high | 2026-03-05 |
| [0035](TODO-0035.md) | Add tabled + terminal_size and create table style helpers | done | high | 2026-03-05 |
| [0036](TODO-0036.md) | Rewrite clean command output | done | high | 2026-03-05 |
| [0037](TODO-0037.md) | Rewrite search command output | done | high | 2026-03-05 |
| [0038](TODO-0038.md) | Rewrite build command output | done | high | 2026-03-05 |
| [0039](TODO-0039.md) | Rewrite check command output | done | high | 2026-03-05 |
| [0040](TODO-0040.md) | Rewrite init command output | done | high | 2026-03-05 |
| [0041](TODO-0041.md) | Rewrite update command output | done | high | 2026-03-05 |
| [0042](TODO-0042.md) | Rewrite info command output | done | high | 2026-03-05 |
| [0043](TODO-0043.md) | Fix tracing levels — distinct debug/trace events with elapsed times | todo | medium | 2026-03-05 |
| [0044](TODO-0044.md) | Cargo.toml metadata and crate optimization | done | high | 2026-03-06 |
| [0045](TODO-0045.md) | cargo-dist initialization and release workflow | done | high | 2026-03-06 |
| [0046](TODO-0046.md) | Homebrew tap via cargo-dist | todo | medium | 2026-03-06 |
| [0047](TODO-0047.md) | npm binary wrapper via cargo-dist | todo | medium | 2026-03-06 |
| [0048](TODO-0048.md) | README install section update | todo | medium | 2026-03-06 |
| [0049](TODO-0049.md) | GitHub Actions CI workflow | done | high | 2026-03-06 |
| [0050](TODO-0050.md) | Fix String null serialization to Arrow NULL | done | high | 2026-03-07 |
| [0051](TODO-0051.md) | Replace panic! in model loading with error propagation | done | high | 2026-03-07 |
| [0052](TODO-0052.md) | Handle unreadable files gracefully in scan | done | high | 2026-03-07 |
| [0053](TODO-0053.md) | Handle symlink escape in scan strip_prefix | done | high | 2026-03-07 |
| [0054](TODO-0054.md) | Handle invalid glob pattern without panicking | done | high | 2026-03-07 |
| [0055](TODO-0055.md) | Add file size limit to scan | done | medium | 2026-03-07 |
| [0056](TODO-0056.md) | Verify .mdvs/ is not a symlink before clean | done | medium | 2026-03-07 |
| [0057](TODO-0057.md) | Refactor build::run() — extract model loading helper | done | medium | 2026-03-07 |
| [0058](TODO-0058.md) | Replace unwrap on path to_str in search | done | medium | 2026-03-07 |
| [0059](TODO-0059.md) | Replace unwrap on JSON serialization in output | done | medium | 2026-03-07 |
| [0060](TODO-0060.md) | Add tests for update command | done | high | 2026-03-07 |
| [0061](TODO-0061.md) | Add test for build validation abort | done | high | 2026-03-07 |
| [0062](TODO-0062.md) | Add test for null value parquet roundtrip | done | high | 2026-03-07 |
| [0063](TODO-0063.md) | Add search edge case tests | done | high | 2026-03-07 |
| [0064](TODO-0064.md) | Add parquet roundtrip tests for complex types | done | medium | 2026-03-07 |
| [0065](TODO-0065.md) | Add tests for table.rs and output.rs | done | low | 2026-03-07 |
| [0066](TODO-0066.md) | Add YAML nesting depth limit | done | medium | 2026-03-07 |
| [0067](TODO-0067.md) | Add frontmatter field count limit | done | medium | 2026-03-07 |
| [0068](TODO-0068.md) | Add context to parquet read error messages | done | low | 2026-03-07 |
| [0069](TODO-0069.md) | Warn when search verbose chunk text is unavailable | done | low | 2026-03-07 |
| [0070](TODO-0070.md) | Extract shared inference logic from init and update | done | low | 2026-03-07 |
| [0071](TODO-0071.md) | Break up monolithic validate() function | done | low | 2026-03-07 |
| [0072](TODO-0072.md) | Escape special characters in search SQL construction | done | medium | 2026-03-07 |
| [0073](TODO-0073.md) | Build violation output goes to stderr instead of stdout | todo | high | 2026-03-07 |
| [0074](TODO-0074.md) | Replace DefaultHasher with stable hash for content_hash | todo | low | 2026-03-07 |
| [0075](TODO-0075.md) | Support array containment queries in --where | todo | medium | 2026-03-07 |
| [0076](TODO-0076.md) | Ergonomic --where queries for field names with spaces | todo | medium | 2026-03-07 |
