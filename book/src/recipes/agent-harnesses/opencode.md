# OpenCode

## Install

```bash
mkdir -p .opencode/skills/mdvs
mdvs scaffold skill > .opencode/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform opencode >> AGENTS.md
```

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded by OpenCode on session start.
- **Snippet**: always-on `AGENTS.md` block telling the agent to prefer `mdvs search` over `Grep`.

## Hooks

OpenCode handles tool events through a TypeScript plugin API rather than shell-command hooks. At the moment, mdvs doesn't ship a verified plugin or hook config for OpenCode, to wire `mdvs hook handle` into OpenCode's plugin events, follow the [OpenCode docs](https://opencode.ai/docs/).

As a harness-independent fallback, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook) runs `mdvs check` on every commit.

## Per-platform notes

- **Skill path**: `.opencode/skills/mdvs/SKILL.md` (native). OpenCode also reads `.claude/skills/` and `.agents/skills/` — you can symlink across if you want a single source of truth shared with another harness.
- **Project rules**: `AGENTS.md` at workspace root. OpenCode also reads `CLAUDE.md` as a Claude Code-compat fallback.

## Sources

- [OpenCode skills](https://opencode.ai/docs/skills/)
- [OpenCode agents](https://opencode.ai/docs/agents/)
- [OpenCode rules](https://opencode.ai/docs/rules/)
