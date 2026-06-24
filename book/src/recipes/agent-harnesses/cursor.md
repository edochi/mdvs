# Cursor

> **Schema-correct, hooks not observed firing in initial smoke test.** The install commands produce output that matches [Cursor's hooks reference](https://cursor.com/docs/hooks) — the flat matcher entry shape, the top-level `"version": 1` field, the snake-case `additional_context` in the runtime envelope. In a live smoke test against a real vault, the hook was **not observed firing** after a markdown edit — likely a wiring bug (matcher path, envelope shape, or config-file location) that needs investigation. The skill and snippet halves work as expected. If you wire mdvs into Cursor and either get the hook firing or can diagnose why it's not, please [open an issue](https://github.com/edochi/mdvs/issues).

For the design intent behind the integration and the runtime story, see the [Agent harnesses overview](../agent-harnesses.md). For copy-paste install steps, read on.

## Install

```bash
mkdir -p .cursor/skills/mdvs .cursor/rules
mdvs scaffold skill > .cursor/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform cursor > .cursor/rules/mdvs.mdc
mdvs scaffold hook --platform cursor >> .cursor/hooks.json
```

Note the snippet uses `>` not `>>` — the Cursor variant is a `.mdc` file with YAML frontmatter at the top (`alwaysApply: true`), so it stands alone in `.cursor/rules/`, not appended to a larger file.

If `.cursor/hooks.json` already exists, **merge by hand**: the snippet emits a complete top-level object with `version: 1` and the `hooks.postToolUse` array; you'll need to union the array contents with anything you already have.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Cursor reads skills from `.cursor/skills/`, `.agents/skills/`, `.claude/skills/`, and `.codex/skills/` — `mdvs scaffold skill --platform cursor` writes to `.cursor/skills/` as the native path.
- **Snippet**: `.cursor/rules/mdvs.mdc` with `alwaysApply: true`. Cursor includes it in every conversation automatically.
- **Validate hook**: after every `Edit` / `Write` / `MultiEdit` on a markdown file inside an mdvs vault, `mdvs hook handle` runs `check` and surfaces violations through Cursor's `additional_context` field. Always exits 0.
- **Search-nudge hook**: same pattern, matching Bash search-tool invocations.

## Per-platform notes

- **Hook envelope shape is different from Claude Code / Codex.** Cursor uses snake-case `additional_context` at the top level, no `hookSpecificOutput` wrapper. The `mdvs hook handle` runtime emits this shape automatically when called with `--platform cursor` — the per-platform JSON shape lives in `cursor/platform.toml`'s envelope template.
- **No user-visible channel.** Cursor's `postToolUse` only has an agent-context field (`additional_context`); there's no equivalent of Claude Code's `systemMessage`. Validation violations reach the agent but not the user UI directly. (Cursor has `user_message` for the permission/deny flow, but that's separate.)
- **Config shape is different too.** Cursor's `hooks.json` uses flat matcher entries (`{ matcher, command }`) — no nested `hooks` array — and includes a top-level `"version": 1` field. The emitted JSON snippet captures this.
- **Project rules**: Cursor also honors `AGENTS.md` at workspace root. If you'd rather paste the universal snippet there: `mdvs scaffold snippet >> AGENTS.md` (without `--platform`).
- **mdvs on PATH**: the hook command is `mdvs hook handle --platform cursor --kind <kind>`. `mdvs` must be available to Cursor's hook subprocess. On macOS, Cursor launched from Spotlight may not see your shell PATH — symlink `mdvs` into `/usr/local/bin/` or install via `cargo install --path crates/mdvs`.

## Sources

- [Cursor skills](https://cursor.com/docs/context/skills)
- [Cursor rules](https://cursor.com/docs/rules)
- [Cursor hooks](https://cursor.com/docs/hooks)
