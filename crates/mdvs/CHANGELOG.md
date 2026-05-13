# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## v0.4.2 - 2026-05-13
#### Miscellaneous Chores
- (**version**) v0.4.1 - (0386285) - release-bot

- - -

## v0.4.1 - 2026-05-13
#### Bug Fixes
- (**release**) publish tomljson alongside mdvs - (ee3f7a9) - edoch

- - -

## v0.4.0 - 2026-05-13
#### Features
- (**example_kb**) enrich with pattern/length/range constraints + Array(Object) - (2cc9fb1) - edoch
- (**infer**) drop Array(Object) fields at inference + invariant 9 - (5525c07) - edoch
- (**mdvs**) function-style type display (TODO-0097 step 7, closes 0096) - (2ef8999) - edoch
- (**mdvs**) nested Arrow Structs from dotted-name leaves (TODO-0097 step 5) - (0126987) - edoch
- (**mdvs**) per-leaf validation by dotted path (TODO-0097 step 4) - (4b36049) - edoch
- (**mdvs**) translator handles dotted names (TODO-0097 step 3) - (cbf3e39) - edoch
- (**mdvs**) reject top-level Object + validate dotted names (TODO-0097 step 2) - (fb9afd4) - edoch
- (**mdvs**) emit one InferredField per leaf (TODO-0097 step 1) - (ba88c7a) - edoch
- (**mdvs**) strict-Float fields reject integers by default - (b98033c) - edoch
- (**mdvs**) schema content hash in build metadata (TODO-0149 Wave B step 14) - (9254846) - edoch
- (**mdvs**) mdvs export-jsonschema command (TODO-0149 Wave B step 9) - (96d33c2) - edoch
- (**mdvs**) --schema flag on init and check (TODO-0149 Wave B step 10) - (66ca07a) - edoch
- (**mdvs**) schema loader + source resolver (TODO-0149 Wave B steps 11+12) - (7c65295) - edoch
- (**mdvs**) add min_length, max_length, pattern constraints (TODO-0149 Wave B step 7) - (243513d) - edoch
- (**mdvs**) preprocessor pipeline + inference-driven preprocess field (TODO-0149 Wave B step 3) - (910df86) - edoch
- (**mdvs**) swap hand-rolled validation for jsonschema (TODO-0149 Wave B step 4+5) - (c52e765) - edoch
- (**mdvs**) add DSL→canonical translator and schema gate (TODO-0149 Wave B step 2) - (9fcc7b3) - edoch
- (**tomljson**) make root wrapper key configurable - (51b869f) - edoch
- (**tomljson**) lift baseline encoding-rules tests (TODO-0149 Wave A step 6) - (9219e41) - edoch
- (**tomljson**) implement deserialize tree walker (TODO-0149 Wave A step 5) - (8b80ff4) - edoch
- (**tomljson**) implement serialize dispatcher (TODO-0149 Wave A step 4) - (3bf9749) - edoch
- <span style="background-color: #d73a49; color: white; padding: 2px 6px; border-radius: 3px; font-weight: bold; font-size: 0.85em;">BREAKING</span>(**types**) function-style type syntax on disk + reject Array(Object) - (d51c7a2) - edoch
#### Bug Fixes
- (**check**) collapse additionalProperties match guard - (7833f4e) - edoch
- (**mdvs**) correct key order on Array/Object field types in mdvs.toml - (bd31c17) - edoch
- (**mdvs**) respect --dry-run when --force is set on init - (37e023d) - edoch
- (**mdvs**) surface YAML→JSON conversion failures (TODO-0149 Wave B step 8) - (d1e9316) - edoch
#### Documentation
- (**TODO-0149**) add Wave B execution plan appendix - (9980ea2) - edoch
- (**TODO-0149**) drop standalone API and $tomljson-null directive - (79d8f43) - edoch
- (**mdvs**) Wave C docs sweep (TODO-0097 step 10) - (1a82ad1) - edoch
- (**mdvs**) refresh specs + mdBook + README for Wave B - (0ce9fd7) - edoch
- (**todos**) close TODO-0155 (function-style syntax shipped) - (95b3def) - edoch
- (**todos**) split TODO-0155 into syntax unification + Array-of-Object - (5dfcfd8) - edoch
- (**todos**) expand TODO-0155 to cover display ↔ serde unification - (0c79bdf) - edoch
- (**todos**) close TODO-0097 + TODO-0149 (all three waves done) - (f1df13b) - edoch
- (**todos**) TODO-0097 — execution plan + resolved design decisions - (9e0a9df) - edoch
- (**todos**) TODO-0155 — reusable type definitions via $defs / $ref - (1413cc1) - edoch
- (**todos**) reopen TODO-0149 — Wave C still pending - (0a4a5bb) - edoch
- (**todos**) close TODO-0149 (Wave B complete; step 13 deferred to 154) - (7bc707a) - edoch
- (**todos**) TODO-0154 (overlay cache) + couple to TODO-0149 step 13 - (1513f99) - edoch
- (**tomljson**) trim apologetic tone from README - (84aa7ec) - edoch
- (**tomljson**) write README.md (TODO-0149 Wave A step 7) - (1bf0a88) - edoch
- (**types**) sweep specs + mdBook for function-style syntax and Array(Object) rejection - (1918522) - edoch
- rewrite AGENTS.md to be provider-agnostic + update for Wave B - (c441076) - edoch
- reframe TODO-0153 as three small composable crates - (fe65d69) - edoch
- add TODO-0152 (layered structure) and TODO-0153 (validation engine) - (15e59da) - edoch
- simplify --schema flag and drop validate/search crate split (TODO-0149) - (900260a) - edoch
- design path-scoped validation + error mapping (TODO-0149) - (df55687) - edoch
- split schema sourcing into reference / import / override (TODO-0149) - (a606108) - edoch
- add crate organization and pipeline architecture (TODO-0149) - (95baaf5) - edoch
- rename tomlschema → tomljson, finalize crate design (TODO-0149) - (41816b0) - edoch
- design tomlschema crate + JSON Schema integration (TODO-0149) - (3cc2838) - edoch
- reprioritize TODOs and mark TODO-0151 done - (dc5b643) - edoch, *Claude*
#### Tests
- (**mdvs**) Array(Object) inference + example_kb closeout (TODO-0097 8+9) - (cac2a94) - edoch
- (**mdvs**) dotted-leaf --where regression coverage (TODO-0097 step 6) - (f1535e7) - edoch
#### Refactoring
- (**mdvs**) rename --schema to --from-jsonschema (init) and --jsonschema (check) - (74d9959) - edoch
- (**mdvs**) delete unreachable per-value validation (TODO-0149 Wave B step 6) - (489d3d4) - edoch
- <span style="background-color: #d73a49; color: white; padding: 2px 6px; border-radius: 3px; font-weight: bold; font-size: 0.85em;">BREAKING</span>(**types**) kebab-case preprocess names + clean error messages - (62894f1) - edoch
#### Miscellaneous Chores
- (**mdvs**) add jsonschema and tomljson deps (TODO-0149 Wave B step 1) - (2bcca1f) - edoch
- (**workspace**) restructure to Cargo workspace, add tomljson skeleton - (158f78d) - edoch
- Wave B closeout — cargo audit/deny + CI workflow - (c14a17b) - edoch

- - -

## v0.3.3 - 2026-04-21
#### Bug Fixes
- serialize widened-to-String values in category inference and validation - (a6ecd2c) - edoch, *Claude*
#### Documentation
- add TODO-0151 and example_kb reproduction for widened category bug - (57c7066) - edoch, *Claude*

- - -

## v0.3.2 - 2026-04-21
#### Features
- add --range/--no-range flags to update reinfer - (ace5899) - edoch, *Claude*
- add range constraints (min/max) on numeric fields - (eab9424) - edoch, *Claude*
#### Bug Fixes
- helpful error messages when mdvs.toml is missing - (199b374) - edoch, *Claude*
#### Documentation
- restructure TODO-0149 into two independent tracks - (38a4e66) - edoch
- add TODO-0149 (JSON Schema delegation) and TODO-0150 (strict mode) - (d15f7c1) - edoch, *Claude*
- document range constraints and --with flag - (7761cc5) - edoch, *Claude*
- update TODO-0008 design for range constraints - (5c8b735) - edoch, *Claude*
#### Refactoring
- replace per-kind reinfer flags with single --with flag - (64f969e) - edoch, *Claude*
#### Miscellaneous Chores
- (**deps**) bump datafusion 53, text-splitter 0.30, toml_edit 0.25 - (c419d3e) - edoch
- rename CLAUDE.md to AGENTS.md with backward-compat symlink - (dafe1c2) - edoch, *Claude*

- - -

## v0.3.1 - 2026-04-16
#### Features
- add `mdvs skill` command to print SKILL.md to stdout - (f76e926) - edoch, *Claude*
#### Bug Fixes
- (**docs**) update Rust edition badge from 2021 to 2024 - (5c185f8) - edoch, *Claude*
#### Documentation
- add SKILL.md for AI agent integration (TODO-0102) - (31014b5) - edoch, *Claude*
#### Continuous Integration
- bump upload-pages-artifact v3→v5 and deploy-pages v4→v5 - (8f0991b) - edoch, *Claude*

- - -

## v0.3.0 - 2026-04-16
#### Features
- refactor update reinfer into subcommand with categorical flags - (1f29ce5) - edoch, *Claude*
- wire InvalidCategory violation into check command - (9b0ee8c) - edoch, *Claude*
- add categorical inference and restructure into submodules - (0e23762) - edoch, *Claude*
- add constraint infrastructure and wire into config - (b776eeb) - edoch, *Claude*
#### Bug Fixes
- raise min_category_repetition default from 2 to 3 - (5c441b2) - edoch, *Claude*
- add --dry-run to reinfer subcommand for CLI ergonomics - (f2d34bd) - edoch, *Claude*
- resolve clippy --all-targets warnings in test code - (c27ae5b) - edoch, *Claude*
#### Documentation
- mark TODO-0147 as done - (6094922) - edoch, *Claude*
- rewrite command specs as developer pipeline docs, clean up shared types - (3e92c5d) - edoch, *Claude*
- add deep-dive specs for inference, storage, and search - (3afce25) - edoch, *Claude*
- add architecture.md as developer code map - (2d1063a) - edoch, *Claude*
- add TODO-0147 for spec restructure as developer code map - (9c30cc3) - edoch, *Claude*
- update mdBook for reinfer subcommand and categorical constraints - (845092e) - edoch, *Claude*
- add TODO-0146 for mdBook updates (constraints + reinfer subcommand) - (6154ada) - edoch, *Claude*
- design constraint architecture for TODO-0006/0008/0010, add TODO-0145 - (dc5853b) - edoch, *Claude*
- update TODO-0144 with scripting language evaluation - (fd4c419) - edoch, *Claude*
- update TODO-0006/0008/0010 designs, add TODO-0143/0144 - (0546d93) - edoch, *Claude*
- update TODO-0006 design (categorical constraints), mark 0100/0118/0142 done - (742e9fd) - edoch
- polish README for promotion - (7c95b6f) - edoch, *Claude*
#### Tests
- add integration tests for categorical constraints pipeline - (1a5fa1b) - edoch, *Claude*
#### Continuous Integration
- bump actions/checkout from v4 to v5 - (52c79d8) - edoch, *Claude*
#### Miscellaneous Chores
- (**deps**) update rustls-webpki 0.103.10 → 0.103.12 - (76d1ba0) - edoch, *Claude*
- upgrade to Rust edition 2024 - (0a6c65e) - edoch, *Claude*
- slim dependencies and add supply chain audit tooling - (1187a70) - edoch, *Claude*

- - -

## v0.2.0 - 2026-03-29
#### Bug Fixes
- (**check**) deduplicate NullNotAllowed violation for required+non-nullable fields - (e6b2458) - edoch, *Claude*
- offset chunk line numbers by frontmatter length - (9870ca5) - edoch, *Claude*
#### Documentation
- update all book output examples to KeyValue table format - (e7c383c) - edoch, *Claude*
- add TODO-0142 (fix chunk line numbers to exclude frontmatter) - (b21c272) - edoch, *Claude*
- add TODO-0141 (global --quiet flag) - (ab6b493) - edoch, *Claude*
- add TODO-0140 (global --dry-run flag) - (dd0bd15) - edoch, *Claude*
- update README output examples to key-value table format - (6c1eb65) - edoch, *Claude*
- close TODOs 0119, 0122, 0133, 0134, 0138, 0139; add 0138, 0139; update 0100 - (eb0440f) - edoch, *Claude*
- mark TODOs 0131, 0132, 0135, 0136, 0137 as done - (cb14a57) - edoch, *Claude*
- add TODO-0137 (flatten Step tree into steps + result) - (fe4bf55) - edoch, *Claude*
- add TODO-0136 (inline auto-update/auto-build) and Justfile - (bfd20e5) - edoch, *Claude*
- add post-migration cleanup TODOs (0134, 0135) - (b4f2c54) - edoch, *Claude*
- update TODO status for Step tree implementation - (c64118a) - edoch
- add incremental checklists to Step tree implementation TODOs - (78366dc) - edoch
- add Step tree architecture design and implementation TODOs - (8b91902) - edoch
- rework book intro and getting-started for directory-aware schema - (dd14032) - edoch
- rework README to show directory-aware schema inference - (3c9ce8b) - edoch
- fix nullable description — all four checks are independent - (03a5124) - edoch
#### Refactoring
- redesign clean output to KeyValue style - (0bdfe7d) - edoch, *Claude*
- redesign search output to KeyValue style - (50293e9) - edoch, *Claude*
- redesign build output to KeyValue style - (730fbb1) - edoch, *Claude*
- fill text output gaps — match JSON fields exactly - (0552f45) - edoch, *Claude*
- redesign check output to KeyValue style - (daf77c0) - edoch, *Claude*
- redesign update output to KeyValue style - (e7ce7fa) - edoch, *Claude*
- redesign info output, tweak KeyValue rendering - (8763b4b) - edoch, *Claude*
- add KeyValue table style, redesign init output - (a7f4b09) - edoch, *Claude*
- redesign init output — skip default constraints, no headers - (81733d1) - edoch, *Claude*
- use Panel for detail rows, fix column width proportions - (dc42fde) - edoch, *Claude*
- sort fields in mdvs.toml, add unit tests, unify fail helpers - (6ade7eb) - edoch, *Claude*
- use untagged serialization for Outcome enum - (5a962f8) - edoch, *Claude*
- flatten Step tree into CommandResult, delete CompactOutcome - (4623b5e) - edoch, *Claude*
- inline auto-update and auto-build to eliminate redundant reads - (6fa9210) - edoch, *Claude*
- delete src/pipeline/ directory - (96f0974) - edoch, *Claude*
- inline all pipeline calls into commands, remove migration helpers - (78c7a0f) - edoch, *Claude*
- begin pipeline cleanup — delete delete_index, move BuildFileDetail - (6d64c18) - edoch
- convert all 7 commands to Step tree architecture - (1243d1c) - edoch
- add Step tree infrastructure and outcome types - (7368642) - edoch
#### Miscellaneous Chores
- add book serve target to Justfile - (a32a0ea) - edoch, *Claude*

- - -

## v0.1.1 - 2026-03-17
#### Bug Fixes
- null values now trigger Disallowed and NullNotAllowed checks - (d6e4484) - edoch
#### Documentation
- add TODO-0114 (mdbook-cmdrun) and TODO-0115 (asciinema) - (35db5d2) - edoch

- - -

## v0.1.0 - 2026-03-16
#### Features
- auto-pipeline redesign — downstream commands auto-run upstream steps - (f1d9921) - edoch
- reject unknown fields in mdvs.toml with deny_unknown_fields - (ae6b70a) - edoch
#### Documentation
- close TODO-0099 - (039ec54) - edoch
- update mdBook for auto-pipeline redesign, add TODO-0113 for progress bar - (d850982) - edoch

- - -

## v0.1.0-rc.4 - 2026-03-15
#### Documentation
- set homepage to mdBook, remove documentation field - (0b2d27c) - edoch
- update README install section with prebuilt binary option - (c9e1742) - edoch
#### Continuous Integration
- add --no-verify to publish, ignore merge commits, remove redundant CI trigger, fix docs link - (538634e) - edoch
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.4 - (37e701b) - release-bot

- - -

## v0.1.0-rc.3 - 2026-03-15
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.3 - (a7c06b7) - release-bot

- - -

## v0.1.0-rc.2 - 2026-03-15
#### Bug Fixes
- include Cargo.lock in pre_bump_hooks - (8daebde) - edoch
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.2 - (754c2f7) - release-bot

- - -

## v0.1.0-rc.1 - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- drop Windows and Intel Mac from release targets - (9a5a768) - edoch
- enable crates.io publishing in bump workflow - (17a082b) - edoch
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.1 - (75d4411) - release-bot
- (**version**) v0.1.0-rc - (413db95) - release-bot
- (**version**) v0.1.0-rc - (8918187) - release-bot
- update Cargo.lock - (5300732) - edoch
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc.4 - 2026-03-15
#### Documentation
- set homepage to mdBook, remove documentation field - (0b2d27c) - edoch
- update README install section with prebuilt binary option - (c9e1742) - edoch
#### Continuous Integration
- add --no-verify to publish, ignore merge commits, remove redundant CI trigger, fix docs link - (538634e) - edoch

- - -

## v0.1.0-rc.3 - 2026-03-15
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.3 - (a7c06b7) - release-bot

- - -

## v0.1.0-rc.2 - 2026-03-15
#### Bug Fixes
- include Cargo.lock in pre_bump_hooks - (8daebde) - edoch
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.2 - (754c2f7) - release-bot

- - -

## v0.1.0-rc.1 - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- drop Windows and Intel Mac from release targets - (9a5a768) - edoch
- enable crates.io publishing in bump workflow - (17a082b) - edoch
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.1 - (75d4411) - release-bot
- (**version**) v0.1.0-rc - (413db95) - release-bot
- (**version**) v0.1.0-rc - (8918187) - release-bot
- update Cargo.lock - (5300732) - edoch
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc.3 - 2026-03-15
#### Bug Fixes
- include Cargo.lock in pre_bump_hooks - (8daebde) - edoch
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.2 - (754c2f7) - release-bot

- - -

## v0.1.0-rc.1 - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- drop Windows and Intel Mac from release targets - (9a5a768) - edoch
- enable crates.io publishing in bump workflow - (17a082b) - edoch
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.1 - (75d4411) - release-bot
- (**version**) v0.1.0-rc - (413db95) - release-bot
- (**version**) v0.1.0-rc - (8918187) - release-bot
- update Cargo.lock - (5300732) - edoch
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc.2 - 2026-03-15
#### Bug Fixes
- include Cargo.lock in pre_bump_hooks - (8daebde) - edoch

- - -

## v0.1.0-rc.1 - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- drop Windows and Intel Mac from release targets - (9a5a768) - edoch
- enable crates.io publishing in bump workflow - (17a082b) - edoch
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc.1 - (75d4411) - release-bot
- (**version**) v0.1.0-rc - (413db95) - release-bot
- (**version**) v0.1.0-rc - (8918187) - release-bot
- update Cargo.lock - (5300732) - edoch
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc.1 - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- drop Windows and Intel Mac from release targets - (9a5a768) - edoch
- enable crates.io publishing in bump workflow - (17a082b) - edoch
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc - (413db95) - release-bot
- (**version**) v0.1.0-rc - (8918187) - release-bot
- update Cargo.lock - (5300732) - edoch
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- add pre_bump_hook to update Cargo.toml version - (dccbe70) - edoch
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- (**version**) v0.1.0-rc - (8918187) - release-bot
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

## v0.1.0-rc - 2026-03-15
#### Features
- validate config invariants on mdvs.toml load - (74ab71a) - edoch, *Claude*
- add ContainsSpaces hint and example_kb edge-case fields - (32d390a) - edoch, *Claude*
- verbose text output with process step lines for init, build, update - (af1b31c) - edoch, *Claude*
- compact JSON output — result-only when no errors - (769173c) - edoch, *Claude*
- add core pipeline abstractions (ProcessingStep, StepOutput trait) - (b8a325e) - edoch, *Claude*
- add nullable field property and permissive field defaults - (e54637d) - edoch, *Claude*
- add example knowledge base for testing - (84908c5) - edoch, *Claude*
- initial implementation - (f9459e2) - edoch, *Claude*
#### Bug Fixes
- (**build**) skip dimension check on full rebuild - (7af03b7) - edoch, *Claude*
- (**ci**) strip directory prefix when extracting cocogitto binary - (8776ceb) - edoch
- (**ci**) grant write permission to claude code review workflow - (c5a21bb) - edoch
- (**ci**) correct mdbook-mermaid version in book workflow - (380a6c6) - edoch
- (**ci**) install mdbook-mermaid in book deployment workflow - (e00847f) - edoch
- (**docs**) correct cog bump --pre syntax to require base level flag - (1e06d71) - edoch, *Claude*
- --set-revision with empty string or "None" clears the revision - (34e610e) - edoch
- send build violation output to stdout via BuildOutcome enum - (a8ea12c) - edoch, *Claude*
- replace DefaultHasher with stable xxh3 hash for content_hash - (2b79f78) - edoch, *Claude*
- escape special characters in search SQL and add field hints - (ec84350) - edoch, *Claude*
- resolve audit findings 0050-0070 - (8df309e) - edoch, *Claude*
#### Documentation
- update release process for branch protection workflow - (ec40e6a) - edoch
- add CI, license, Rust, and docs badges to README - (fae1007) - edoch
- enforce feature branch workflow in CLAUDE.md and commit skill - (c5d28e4) - edoch
- close TODO-0095, GitHub Pages deployment working - (7e82e76) - edoch
- add TODO-0112 for JSON output documentation - (2dc0e0f) - edoch
- update CLAUDE.md for internal column redesign and current architecture - (7805bc7) - edoch
- add TODO-0110, TODO-0111 and update dependency chains - (2290d2a) - edoch, *Claude*
- add TODO-0109 for cleaning up DataFusion error messages - (30553d8) - edoch, *Claude*
- add dependency chain for TODO-0100 and TODO-0101 on TODO-0099 - (d3b6dc9) - edoch, *Claude*
- add TODO-0106 (link graph) and TODO-0107 (pre-commit hook) - (568a1cb) - edoch, *Claude*
- close TODO-0029, delete old book, add docs link to README - (6b6dde5) - edoch, *Claude*
- add CI recipe placeholder and TODO-0105 - (fc803a2) - edoch, *Claude*
- write Obsidian recipe page - (decee47) - edoch, *Claude*
- add TODO-0104 for bare filename in --where - (bb29714) - edoch, *Claude*
- write search guide and configuration reference - (ce58512) - edoch, *Claude*
- add commands hub page and nest command pages as sub-items - (d760a49) - edoch, *Claude*
- write configuration reference page - (46e33f7) - edoch, *Claude*
- add TODOs for agent skill distribution and config invariant validation - (164e177) - edoch, *Claude*
- write clean command page - (5a0efa5) - edoch, *Claude*
- write info command page - (b0074e3) - edoch, *Claude*
- write search command page - (8f9d67f) - edoch, *Claude*
- add TODOs for pipeline redesign, output format, and markdown output - (30807b0) - edoch, *Claude*
- write build command page - (5461e8f) - edoch, *Claude*
- write update command page - (cb4a531) - edoch, *Claude*
- write check command page - (5865443) - edoch, *Claude*
- write init command page and add code-editing/commit skills - (d9e6e70) - edoch, *Claude*
- write search & indexing concept page, fix cosine similarity description, add TODO-0098 - (a42aacf) - edoch, *Claude*
- write validation concept page and standardize violation name style - (b4e1fe1) - edoch, *Claude*
- write schema inference concept page - (9663e55) - edoch, *Claude*
- write types & widening concept page, add calibration to experiment-1 - (27cf63b) - edoch, *Claude*
- add mdbook-mermaid support and TODO-0095 for GitHub Pages deployment - (52b1bc2) - edoch, *Claude*
- split concepts into sub-pages and update book plan - (c993728) - edoch, *Claude*
- write getting-started page and add frontmatter section to introduction - (c2865c0) - edoch, *Claude*
- check off introduction in TODO-0029 - (108ca00) - edoch, *Claude*
- write book introduction page - (c970a0b) - edoch, *Claude*
- expand TODO-0029 with full checklist and mark in-progress - (fec9590) - edoch, *Claude*
- scaffold mdBook site and add book skill - (0fb94a1) - edoch, *Claude*
- update TODO-0029 — book at repo root, example_kb examples, high priority - (6a5629a) - edoch, *Claude*
- add TODO-0094 — hard error on scan safety limits - (1505078) - edoch, *Claude*
- update README tagline with visual contrast - (557a1b2) - edoch, *Claude*
- close TODOs 0091, 0092, 0093 — consistent output rules - (7cbfc9e) - edoch, *Claude*
- split TODO-0091 into 0092 (compact JSON) and 0093 (verbose text) - (d192b7c) - edoch, *Claude*
- close TODO-0078 — structured error output complete - (3e666dd) - edoch, *Claude*
- close TODO-0080 and pause TODO-0088 - (647e143) - edoch, *Claude*
- add TODO-0089 — warn on stale index during search - (7ae86d8) - edoch, *Claude*
- close TODO-0031 — covered by in-repo example_kb/ - (67e0d3b) - edoch, *Claude*
- finalize pipeline design in TODOs 0078-0088 - (5fda2ba) - edoch, *Claude*
- update TODO-0048 scope and drop blockers from TODO-0046/0047 - (d6b40ab) - edoch, *Claude*
- add TODOs 0078-0088 for structured pipeline output rework - (f37983f) - edoch, *Claude*
- update TODO-0076 with ContainsSpaces hint design - (c882ca0) - edoch, *Claude*
- update TODO-0075 with DataFusion array/struct query findings - (485b3ff) - edoch, *Claude*
- add TODO-0077 and update TODO-0074 with implementation details - (9698882) - edoch, *Claude*
- update TODO-0005 with nullable field design and bump to high priority - (2389362) - edoch, *Claude*
- add audit TODOs 0072-0075 from follow-up review - (6f48a4d) - edoch, *Claude*
- mark audit TODOs 0050-0070 as done - (c0b6e2c) - edoch, *Claude*
- add audit TODOs 0050-0071 from code review - (f38ff57) - edoch, *Claude*
#### Tests
- add array containment query test and document --where patterns - (b184323) - edoch, *Claude*
#### Continuous Integration
- add workflow_dispatch bump workflow with deploy key - (a1ba634) - edoch
- add GitHub Actions workflow for mdBook deployment to Pages - (0a1e7ff) - edoch
#### Refactoring
- move internal column prefix from storage to search view - (1aaea6b) - edoch, *Claude*
- rename widen() to FieldType::from_widen() - (af9413c) - edoch, *Claude*
- improve update output and add step labels to process lines - (cd4fedd) - edoch, *Claude*
- remove check_result from BuildCommandOutput - (bfd6973) - edoch, *Claude*
- rework update command to use step-based pipeline model - (1612967) - edoch, *Claude*
- rework init command to use step-based pipeline model - (5e061dd) - edoch, *Claude*
- rework build command to use step-based pipeline model - (9b54102) - edoch, *Claude*
- rework search command to use step-based pipeline model - (0b4dbb6) - edoch, *Claude*
- rework info command to use step-based pipeline model - (b3e633e) - edoch, *Claude*
- rework clean command to use step-based pipeline model - (8b97303) - edoch, *Claude*
- rework check command to use step-based pipeline model - (5d08bee) - edoch, *Claude*
- break up monolithic validate() into focused helpers - (b516dd4) - edoch, *Claude*
#### Miscellaneous Chores
- replace cargo-release with cog bump for releases - (d3d8262) - edoch, *Claude*

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).