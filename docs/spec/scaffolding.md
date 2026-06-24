# Scaffolding

How mdvs integrates with agent harnesses (Claude Code, Codex, Cursor, OpenCode, Antigravity) and how to add a new platform.

This page documents the internal architecture. User-facing instructions live in [book/src/recipes/agentic-harnesses-and-agentic-ides.md](../../book/src/recipes/agentic-harnesses-and-agentic-ides.md).

## Overview

Three install-time commands write content to disk in the harness's expected location:

- `mdvs scaffold skill [--platform <name>]` — emits a comprehensive `SKILL.md` (Agent Skills standard).
- `mdvs scaffold snippet [--platform <name>]` — emits a project-rules block to paste into `AGENTS.md` / `CLAUDE.md` / `.cursor/rules/`.
- `mdvs scaffold hook --platform <name>` — emits a per-platform PostToolUse hook config (JSON snippet) that the harness's settings file ingests.

One runtime command handles every hook invocation:

- `mdvs hook handle --platform <name> --kind <validate|search-nudge>` — reads the harness's stdin JSON, walks up to find `mdvs.toml`, runs the requested logic, writes a platform-shaped envelope to stdout, exits 0.

All four commands share the same per-platform data: `scaffolding/platforms/<name>/platform.toml`. Adding a new harness is adding one toml file; no Rust changes.

## Directory layout (`crates/mdvs/scaffolding/`)

```
scaffolding/
├── skill/SKILL.md                       — universal skill body, harness-agnostic
├── snippet/
│   ├── agents-md.md                     — universal AGENTS.md / CLAUDE.md snippet
│   └── cursor-rules.mdc                 — same body wrapped in Cursor `.mdc` frontmatter
└── platforms/
    ├── claude-code/platform.toml
    ├── codex/platform.toml
    ├── cursor/platform.toml
    ├── opencode/platform.toml           — skill + snippet only, no [hooks]
    └── antigravity/platform.toml        — skill + snippet only, no [hooks]
```

The entire `scaffolding/` tree is bundled into the binary at build time via `include_dir!`. No disk access at runtime; users get a single binary that knows about every platform.

## `platform.toml` schema

```toml
[meta]
name = "..."                # canonical name, must match the directory name
display_name = "..."        # human-readable label for help text
documentation_url = "..."   # optional, surfaced for users to verify shape

[skill]
install_path = "..."        # where the harness expects `SKILL.md`

[snippet]
target_file = "..."         # where to append/write the snippet
body = "agents-md"          # which bundled body to use; values: agents-md | cursor-rules

[hooks]                     # OPTIONAL — omit for harnesses without a shell-command hook surface
config_path = "..."         # path to the harness's hooks file
config_format = "json"      # current valid: "json" (only)

[hooks.envelope]
template = """             # JSON, parsed at load time, with <<MARKER>> placeholders
{ ... }
"""

[hooks.config]
template = """             # JSON, parsed at load time, with <<MARKER>> placeholders
{ ... }
"""
```

The `[hooks]` section is what makes a platform "hook-capable." Platforms without it (OpenCode, Antigravity) get full skill + snippet support, but `mdvs scaffold hook --platform <name>` and `mdvs hook handle --platform <name>` both refuse with a pointer at the recipe page.

## Template substitution (`scaffold/template.rs`)

`platform.toml` declares the JSON shapes the harness expects. Variable parts (the agent message, the user message, the install-time command strings) are filled in at the point of use via `<<MARKER>>` placeholders.

### Marker syntax

A marker is a JSON string whose **entire content** matches `<<NAME>>` where `NAME` is an uppercase identifier. The full-string-match rule means `"hello <<NAME>>"` is **NOT** a marker — it's a literal string with text that happens to contain angle brackets. This avoids the escaping and partial-substitution edge cases of `printf`-style templating.

### Substitution rules

Given `vars: HashMap<&str, Option<String>>`:

| `vars[NAME]` | Behavior |
|---|---|
| `Some(value)` | Replace the marker string with the value as a JSON string node. |
| `None` | **Prune**: remove the parent key (in objects) or array element. |
| Not present in vars | Same as `None`. |

The prune-on-`None` rule handles per-kind variance cleanly. Example: Claude Code's envelope template has both `<<MSG>>` and `<<USER_MSG>>`. For `validate`, both are populated → full envelope. For `search-nudge`, only `<<MSG>>` is populated → the `<<USER_MSG>>` marker is pruned, taking `"systemMessage"` with it. The agent sees the tip in `additionalContext`; the user UI stays quiet.

Cursor's envelope template doesn't reference `<<USER_MSG>>` at all (Cursor's `postToolUse` has no user channel). Whether mdvs passes `USER_MSG=Some(...)` or `None` makes no difference — same output either way.

### Implementation

```rust
pub fn substitute(template: &Value, vars: &HashMap<&str, Option<String>>) -> Value;
```

Walks the parsed `serde_json::Value` tree, returning a new tree with markers replaced and pruned keys/elements removed. No string concatenation, no escaping required from the template author — substitution operates on JSON nodes, not raw text.

Tests covering all the rules: `scaffold::template::tests` (14 tests).

## The two consumers

### `mdvs scaffold hook` (install-time)

```text
$ mdvs scaffold hook --platform claude-code
```

Loads `Platform`, builds two vars:

- `<<COMMAND_VALIDATE>>` → `"mdvs hook handle --platform claude-code --kind validate"`
- `<<COMMAND_SEARCH>>` → `"mdvs hook handle --platform claude-code --kind search-nudge"`

Substitutes into `hooks.config` template. Injects a `_comment` field at the top of the resulting object explaining where to merge it. Emits to stdout (clean for piping); install-path hint goes to stderr.

For platforms without `[hooks]`: refuses with a pointer that still directs users to `mdvs scaffold skill|snippet`.

### `mdvs hook handle` (runtime)

```text
$ echo '{"tool_input":{"file_path":"kb/note.md"},"cwd":"/path"}' \
  | mdvs hook handle --platform claude-code --kind validate
```

For `--kind validate`:

1. Read stdin JSON, extract `tool_input.file_path`. Silent if absent or not `.md`.
2. Walk up from the file's directory looking for `mdvs.toml`. Silent if no vault.
3. Run `cmd::check::run` on the vault directly (no subprocess; no shell).
4. If no violations and no error → silent exit 0.
5. Render the result twice: `--output markdown` for the agent channel (uncapped), `--output pretty` for the user channel (capped at `MAX_USER_LINES = 15` with a `...` truncation marker).
6. Append a skill pointer to the agent markdown.
7. Build vars: `<<MSG>>` = agent markdown, `<<USER_MSG>>` = pretty (`Some` for validate; `None` would prune).
8. Substitute into `hooks.envelope`. Emit to stdout, exit 0.

For `--kind search-nudge`:

1. Read stdin JSON, extract `cwd` (or `pwd` fallback).
2. Walk up to find `mdvs.toml`. Silent if no vault.
3. Match `tool_input.command` against search-tool patterns (`grep`, `rg `, `ripgrep`, `find `, `fd `, `fdfind `, `ag `, `ack `, `git grep`).
4. Build vars: `<<MSG>>` = the static tip, `<<USER_MSG>>` = `None` (search-nudge stays out of the user UI).
5. Substitute into `hooks.envelope`. Emit to stdout, exit 0.

The hook **never blocks** (always exits 0). Violations and tips surface to the agent through `additionalContext`-style channels; the agent decides what to do next. The "warning, not block" design intentionally keeps mdvs out of the harness's permission flow — the schema is meant to evolve with the KB.

For platforms without `[hooks]`: refuses with the same pointer as `mdvs scaffold hook`.

## Adding a new platform

Three categories, ordered by ease.

### A) New harness with a Claude-Code-style hook config

Examples of what would qualify: a hypothetical harness that uses the same `{"hooks": {"<event>": [{"matcher": "...", "hooks": [{"type": "command", "command": "..."}]}]}}` nesting but at a different config path or with a different event-name capitalization.

Steps:

1. Create `scaffolding/platforms/<name>/platform.toml` with the right `[meta]`, `[skill]`, `[snippet]`, `[hooks]` sections.
2. Copy `claude-code/platform.toml`'s `[hooks.envelope]` and `[hooks.config]` templates as a starting point; adjust event names and matcher strings to match the harness's docs.
3. `cargo test --features testing-mocks scaffold::platform` — verifies the toml parses cleanly and the template is valid JSON. Add a per-platform test if the new shape has distinctive landmarks.

No Rust code changes needed.

### B) New harness with a completely different JSON shape

Cursor was the original test case: snake-case `additional_context` at the top level, no `hookSpecificOutput` wrapper, no `systemMessage`-equivalent, flat matcher entries on the config side, top-level `"version": 1`.

Same three steps as (A), but the templates are written from scratch to match the harness's actual schema. The `<<MSG>>` and `<<USER_MSG>>` markers can appear anywhere in the envelope template (or not at all, if the harness has no equivalent channel). The `<<COMMAND_VALIDATE>>` and `<<COMMAND_SEARCH>>` markers are required somewhere in the config template (otherwise `mdvs scaffold hook` would emit an unfilled snippet).

Still no Rust code changes.

### C) New harness with a non-shell hook surface

OpenCode (TypeScript plugin API) and Antigravity (undocumented post-rebrand) are current examples. There's no JSON-config + shell-command pathway for mdvs to slot into.

Steps:

1. Create `scaffolding/platforms/<name>/platform.toml` with `[meta]`, `[skill]`, `[snippet]` only — omit `[hooks]` entirely.
2. `mdvs scaffold skill` and `mdvs scaffold snippet` will work; `mdvs scaffold hook --platform <name>` and `mdvs hook handle --platform <name>` will refuse with a helpful pointer.
3. If/when the harness adds a documented shell-command hook surface, fill in `[hooks]` to upgrade to category (A) or (B).

### What `<<MARKER>>` names are available

Hard-coded set, defined in `cmd/hook/handle.rs::build_envelope` (envelope) and `cmd/scaffold/hook.rs::build_config` (config):

| Marker | Available in | Source |
|---|---|---|
| `<<MSG>>` | envelope | agent-context message (markdown violation report + skill pointer for validate; static tip for search-nudge) |
| `<<USER_MSG>>` | envelope | user-visible message (pretty render of violations, capped). `None` for search-nudge (causes prune). |
| `<<COMMAND_VALIDATE>>` | config | `"mdvs hook handle --platform <name> --kind validate"` |
| `<<COMMAND_SEARCH>>` | config | `"mdvs hook handle --platform <name> --kind search-nudge"` |

Adding a new marker requires a small Rust change in the consumer that fills it; the template engine itself doesn't know about marker names.

## Rust shape (`crates/mdvs/src/scaffold/`)

```
scaffold/
├── mod.rs        — re-exports + SCAFFOLDING: Dir<'_> (the include_dir bundle)
├── platform.rs   — Platform, HooksConfig, deserialize-with for templates
└── template.rs   — substitute()
```

`Platform` is a plain struct loaded from `platform.toml`. No enum (platforms aren't a fixed set), no `dyn Trait` (no library-style downstream extension; this is a binary). New harnesses come from new toml files, never from new Rust types. The project's "enum dispatch, no `dyn Trait`" rule (from [architecture.md](architecture.md#enum-dispatch-pattern)) still applies to concerns where finiteness matters (backends, embedders, value stages); platforms aren't one of those.

## Why `mdvs hook handle` lives in Rust and not in shell scripts

mdvs is already a cross-platform Rust binary. The shell-script approach we shipped initially (six `.sh` files across three platforms) only worked on POSIX systems and required `jq` on `PATH` — a real blocker for Windows users and a friction point even on Mac/Linux. Pulling the logic into a single mdvs subcommand:

- Eliminates the per-OS implementation matrix; one Rust build target serves every OS mdvs already supports.
- Drops the `jq` runtime dependency.
- Lets the install-time `command:` field in the harness's settings file be a one-liner pointing at `mdvs hook handle` — no separate script files to install, no shell-escaping concerns.
- Centralizes the hook contract in one testable place. Bug fixes ship via a normal mdvs release rather than per-user shell-script edits.

The trade-off is that the per-harness JSON shape now lives inside the binary (via the embedded `platform.toml` files) instead of in editable shell scripts. The template-driven design above mitigates this: users (and contributors) can override by adding a new `platform.toml` to the source tree, no Rust touch required.
