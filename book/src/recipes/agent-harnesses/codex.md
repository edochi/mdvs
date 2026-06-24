# Codex

## Install

```bash
mkdir -p .agents/skills/mdvs
mdvs scaffold skill > .agents/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform codex >> AGENTS.md
```

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded from `.agents/skills/mdvs/SKILL.md` (the cross-harness Agent Skills convention).
- **Snippet**: always-on `AGENTS.md` block telling the agent to prefer `mdvs search` over `Grep`.

## Hooks

mdvs doesn't ship a verified Codex hook config. To wire `mdvs hook handle` into Codex's PostToolUse mechanism, follow the [Codex hooks docs](https://developers.openai.com/codex/hooks).

As a harness-independent fallback, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook) runs `mdvs check` on every commit.

## Per-platform notes

- **Skill path**: `.agents/skills/mdvs/SKILL.md` (Codex's canonical path per [their skills docs](https://developers.openai.com/codex/skills/) — same path Cursor and Antigravity also honor).
- **Project rules**: `AGENTS.md` at workspace root. `AGENTS.override.md` takes precedence if present.
- **mdvs on PATH**: `mdvs` must be available to any subprocess Codex runs.

## Sources

- [Codex skills](https://developers.openai.com/codex/skills/)
- [Codex hooks](https://developers.openai.com/codex/hooks)
- [Codex AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md)
