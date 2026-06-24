# Agent harnesses

mdvs ships first-class wiring for agents working in markdown knowledge bases — Claude Code, Codex, Cursor, OpenCode, and Antigravity. The same `mdvs` binary that validates and searches your KB also runs the harness's PostToolUse hooks, so each integration is two commands and one config snippet, not a shell-script install with `jq` dependencies. Per-platform JSON shapes live as data, not Rust — adding a new harness is one toml file.

The rest of this page explains the shape of the integration, what to expect at runtime, and how to extend mdvs to a harness that isn't in the supported list. For copy-paste install steps, see the per-harness pages in the left nav.

## The shape

Three artifacts get installed in each project; one runtime command handles every hook invocation.

**Install-time** (run once, when you set up the project):

- **`mdvs scaffold skill`** — emits the bundled `SKILL.md` (the [Agent Skills standard](https://agentskills.io) — same format Claude Code, Codex, OpenCode, Cursor, and Antigravity all support). Pipe to your harness's skill directory. The skill explains mdvs to the agent: when to call which command, how to interpret violations, the "warning, not block" rule.
- **`mdvs scaffold snippet`** — emits a short project-rules block (~15 lines). Paste into `AGENTS.md` / `CLAUDE.md` / `.cursor/rules/mdvs.mdc` so it's always in the agent's context, not waiting for skill activation.
- **`mdvs scaffold hook --platform <name>`** — emits a JSON snippet for the harness's hooks config file. The snippet's `command:` fields call `mdvs hook handle` directly. No shell scripts, no `jq`, works the same on Mac / Linux / Windows because mdvs is a cross-platform Rust binary.

**Runtime** (called automatically after every Edit / Write / Bash by the agent):

- **`mdvs hook handle --platform <name> --kind <validate|search-nudge>`** — reads the harness's stdin JSON, walks up from the edited file (or current directory) to find an `mdvs.toml`, runs the relevant check (validate frontmatter, or pattern-match the bash command), and writes a platform-shaped envelope to stdout. **Always exits 0** — violations and tips surface to the agent through the harness's model-context channel; mdvs never rejects an edit at the harness layer.

The "data, not Rust" half: every per-platform difference (where each file goes, what JSON shape each harness expects on stdout, which event names and matchers to use) lives in a single toml file per platform under `crates/mdvs/scaffolding/platforms/<name>/platform.toml`. mdvs reads it at runtime; the actual JSON wrapping happens via template substitution. **The Cursor envelope and the Claude Code envelope are structurally different** — Cursor uses flat `additional_context`, Claude Code wraps it in `hookSpecificOutput` — and mdvs supports both from one Rust codepath because the templates carry the shape.

## How violations reach the agent

When the agent edits a markdown file in your vault:

1. The harness's PostToolUse hook fires the configured `mdvs hook handle` command.
2. mdvs reads the tool-call payload, walks up to find `mdvs.toml`. If the edit happened outside any vault, the hook stays silent.
3. mdvs runs `check` on the vault. If the file is clean, the hook stays silent (no noise on the happy path).
4. If there are violations, mdvs writes the platform's envelope JSON to stdout. The harness reads it and surfaces the markdown body to the agent through the appropriate channel (`additionalContext` for Claude Code, `additional_context` for Cursor, etc.).
5. The agent sees the violation and reacts on its next turn — per the [schema-evolution loop](https://github.com/edochi/mdvs/blob/main/crates/mdvs/scaffolding/skill/SKILL.md): if it's a mistake, fix the file; if it's intentional (KB evolving), surface the deviation to the user and propose updating `mdvs.toml`.

A separate **search-nudge** hook fires after every Bash command that runs `grep` / `rg` / `find` / `ag` / `ack` / `fd` / `git grep`. If the agent's cwd is inside an mdvs vault, the hook surfaces a one-line tip suggesting `mdvs search` (which understands meaning and supports `--where` filtering on frontmatter). Like validate, it's non-blocking — the agent decides whether to switch tools.

## Per-platform support

| Platform | Skill | Snippet | Hooks | End-to-end tested |
|---|---|---|---|---|
| [Claude Code](agent-harnesses/claude-code.md) | ✓ | ✓ | ✓ | **Yes — full loop verified** |
| [Codex](agent-harnesses/codex.md) | ✓ | ✓ | ✓ | Built to docs only — hook firing not confirmed in practice |
| [Cursor](agent-harnesses/cursor.md) | ✓ | ✓ | ✓ | Built to docs only — hooks did not fire in initial smoke test |
| [OpenCode](agent-harnesses/opencode.md) | ✓ | ✓ | — (TypeScript plugin bridge; see page) | Bridge plugin did not fire in initial smoke test |
| [Antigravity](agent-harnesses/antigravity.md) | ✓ | ✓ | — (project-level hooks not supported upstream) | skill + snippet verified |

**Reality check.** Only Claude Code's hook path has been verified end-to-end in practice. Codex, Cursor, and OpenCode were built against each harness's published documentation (envelope shape, event names, config file path) and the install commands produce schema-correct output — but in initial smoke tests the hooks were either not observed firing (Cursor, OpenCode) or had unclear behaviour (Codex). The skill and snippet halves work everywhere; the hook half needs more investigation per harness, and PRs that diagnose specific failures are extremely welcome. Pick your harness in the left nav for the install steps and the per-platform caveats.

## Extending to a new harness

If your harness isn't in the list, the path is small and almost always doesn't require Rust changes. Per-platform behavior lives in a single toml file; mdvs reads it at runtime. Walk-through below.

**Scenario.** You're using a (made-up) harness called `FooAgent`. Its docs say PostToolUse hooks live in `.foo/config.json`, the event key is `"AfterEdit"`, and the runtime envelope looks like:

```json
{
  "hookSpecificOutput": {
    "hookEventName": "AfterEdit",
    "additionalContext": "<message>"
  }
}
```

FooAgent reads skills from `.foo/skills/<name>/SKILL.md` and honors `AGENTS.md` as the project-rules file. No user-visible warning channel — only `additionalContext` for the agent.

### Step 1 — Create the directory

```
crates/mdvs/scaffolding/platforms/foo-agent/
└── platform.toml
```

### Step 2 — Write `platform.toml`

```toml
[meta]
name = "foo-agent"
display_name = "FooAgent"
documentation_url = "https://example.com/foo-agent/docs"

[skill]
install_path = ".foo/skills/mdvs/SKILL.md"

[snippet]
target_file = "AGENTS.md"
body = "agents-md"

[hooks]
config_path = ".foo/config.json"
config_format = "json"

[hooks.envelope]
# Note: no <<USER_MSG>> marker because FooAgent has no user-visible
# channel for postToolUse. mdvs's USER_MSG variable will be ignored.
template = """
{
  "hookSpecificOutput": {
    "hookEventName": "AfterEdit",
    "additionalContext": "<<MSG>>"
  }
}
"""

[hooks.config]
template = """
{
  "hooks": {
    "AfterEdit": [
      { "matcher": "Edit|Write",
        "hooks": [{ "type": "command", "command": "<<COMMAND_VALIDATE>>" }] },
      { "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "<<COMMAND_SEARCH>>" }] }
    ]
  }
}
"""
```

### Step 3 — Rebuild mdvs

Because the scaffolding tree is bundled into the binary via `include_dir!` at compile time, you need to rebuild:

```bash
cargo build --release -p mdvs
```

### Step 4 — Verify

```bash
mdvs scaffold hook --platform foo-agent
# → emits the JSON snippet with mdvs hook handle commands filled in
mdvs scaffold skill --platform foo-agent
# → suggests installing to .foo/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform foo-agent
# → emits the agents-md body, suggests appending to AGENTS.md
```

That's it. No Rust changes. Submit a PR adding the `platform.toml` — review is a quick read of the file against the harness's documented schema.

### Three categories of new platforms

The example above is the most common case (a new harness with a JSON-shaped hooks config + skill path). Two other cases:

- **The harness uses a completely different JSON shape** (e.g., flat matcher entries, snake_case field names, a top-level `version` field). Still no Rust changes — just write the templates from scratch to match the harness's actual schema. Cursor was the test case for this: its envelope is `{"additional_context": "..."}` at the top level (no wrapper) and its config has `"version": 1` + flat matcher entries (no nested `hooks` array). Both shapes live entirely in the templates in `cursor/platform.toml`.
- **The harness has no shell-command hook surface** (e.g., it requires a TypeScript plugin like OpenCode does, or its hook spec is undocumented like Antigravity post-rebrand). Omit the `[hooks]` table entirely. `mdvs scaffold skill` and `mdvs scaffold snippet` still work; `mdvs scaffold hook --platform <name>` and `mdvs hook handle --platform <name>` refuse with a pointer that still steers users at the skill/snippet install.

### When the template approach can't accommodate your harness

The current substitution syntax handles JSON shapes where the variable parts are string values (`<<MARKER>>` placeholders that get replaced with strings). It doesn't handle:

- Harnesses that require a non-JSON config (e.g., a custom DSL, a binary protocol, a YAML-only file)
- Configs that need substitutions of structural elements (an array of N items, a numeric value, etc.)
- Runtime envelopes that aren't valid JSON

If your harness needs one of these, **open an issue at [github.com/edochi/mdvs/issues](https://github.com/edochi/mdvs/issues)** with the harness's schema. Substitution can grow to cover more cases without breaking the existing platforms — the right way to shape that extension is best worked out against a real example.

The full internals of the scaffolding subsystem (the substitution algorithm, the prune-on-`None` rule, the bundled directory layout, the `Platform` struct) are documented in [`docs/spec/scaffolding.md`](https://github.com/edochi/mdvs/blob/main/docs/spec/scaffolding.md) — the spec is the right read if you're touching mdvs internals beyond a new `platform.toml`.

## Pre-commit hook (git users)

Independent of the agent-harness story, you can also run `mdvs check` as a git pre-commit hook. Catches frontmatter violations before they reach the repo, harness-independent.

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: mdvs-check
        name: mdvs check
        entry: mdvs check --no-update
        language: system
        pass_filenames: false
```

For CI-side validation, see the [CI recipe](./ci.md).

## What's tested

At the time of writing, all five integrations have been **installed and tried in practice** against real vaults. Results vary by integration:

- **Claude Code** — **full end-to-end loop verified**: edit a file with a bogus frontmatter value, the hook fires, the agent receives the violation via `additionalContext`, the user receives the pretty render via `systemMessage`. The schema-evolution loop path is also verified. This is the only harness where the hook half is known to work.
- **Antigravity CLI** — skill + snippet verified to be picked up by the harness. **Hooks not supported by mdvs for Antigravity** because Antigravity only ships user-level (not project-level) hooks; `mdvs scaffold hook --platform antigravity` refuses with a pointer.
- **Codex** — install commands produce output matching the [Codex hooks reference](https://developers.openai.com/codex/hooks). In a live smoke test the hook **firing status was unclear** (no observable feedback in either direction). Treat as untested in practice until someone can confirm.
- **Cursor** — install commands produce output matching the [Cursor hooks reference](https://cursor.com/docs/hooks). In a live smoke test the hook was **not observed firing**. Likely a wiring bug (matcher path, envelope shape, or config-file location) — needs investigation.
- **OpenCode** — skill + snippet correct. The [reference TypeScript bridge plugin](./agent-harnesses/opencode.md#workaround-typescript-bridge-plugin) was **not observed firing** in a live smoke test. Likely the plugin loader, the `tool.execute.after` event signature, or the prompt injection path needs adjustment — needs investigation.
- **Windows** — architecturally supported (mdvs is a cross-platform Rust binary; no shell or `jq` dependency anywhere) but **not smoke-tested** on Windows.

The skill and snippet halves work everywhere they were tried; the hook half is the part that varies. If you wire mdvs into one of the unverified configurations and either get it working or hit a specific failure mode, [open an issue](https://github.com/edochi/mdvs/issues) — schema mismatches and wiring bugs are real bugs to fix.
