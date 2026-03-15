# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

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