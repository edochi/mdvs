# Cursor

## Install

```bash
mkdir -p .cursor/skills/mdvs .cursor/rules
mdvs scaffold skill > .cursor/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform cursor > .cursor/rules/mdvs.mdc
```

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Cursor reads skills from `.cursor/skills/`, `.agents/skills/`, `.claude/skills/`, and `.codex/skills/` — `mdvs scaffold skill --platform cursor` writes to `.cursor/skills/` as the native path.
- **Snippet**: `.cursor/rules/mdvs.mdc` with `alwaysApply: true`. Cursor includes it in every conversation automatically.

## Hooks

mdvs doesn't ship a verified Cursor hook config. To wire `mdvs hook handle` into Cursor's PostToolUse mechanism, follow the [Cursor hooks docs](https://cursor.com/docs/hooks).

As a harness-independent fallback, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook) runs `mdvs check` on every commit.

## Per-platform notes

- **Project rules**: Cursor also honors `AGENTS.md` at workspace root. If you'd rather paste the universal snippet there: `mdvs scaffold snippet >> AGENTS.md` (without `--platform`).
- **mdvs on PATH**: `mdvs` must be available to any subprocess Cursor runs. On macOS, Cursor launched from Spotlight may not see your shell PATH — symlink `mdvs` into `/usr/local/bin/` or install via `cargo install --path crates/mdvs`.

## Sources

- [Cursor skills](https://cursor.com/docs/context/skills)
- [Cursor rules](https://cursor.com/docs/rules)
- [Cursor hooks](https://cursor.com/docs/hooks)
