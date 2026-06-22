# OpenCode

> **Skill and snippet only — no hooks.** OpenCode's hook surface is the TypeScript plugin API (`tool.execute.before` / `tool.execute.after`), not a shell-command config file. mdvs's `scaffold hook` command refuses for OpenCode with a pointer at this page. Skill and snippet still install normally.

For the design intent behind the integration and the runtime story, see the [Agent harnesses overview](../agent-harnesses.md). For copy-paste install steps, read on.

## Install

```bash
mkdir -p .opencode/skills/mdvs
mdvs scaffold skill > .opencode/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform opencode >> AGENTS.md
```

OpenCode reads skills from `.opencode/skills/` natively, plus `.claude/skills/` and `.agents/skills/` for cross-harness compatibility. We use the native path as the default.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded by OpenCode on session start.
- **Snippet**: always-on `AGENTS.md` block telling the agent to prefer `mdvs search` over `Grep`.
- **Hooks**: not installed by mdvs. If you want post-edit validation feedback through OpenCode, you can either:
  - Write a small OpenCode TypeScript plugin that shells out to `mdvs hook handle --platform claude-code --kind validate` (the Claude Code envelope works for the agent-side display in OpenCode too, since OpenCode reads `additionalContext` from any compatible envelope). The plugin would handle the OpenCode-specific glue.
  - Use a community shell-command-bridge package like [OpenCode-Hooks](https://github.com/KristjanPikhof/OpenCode-Hooks) which adds a `hooks.yaml` shell layer to OpenCode. With that installed, you can wire `mdvs hook handle --platform claude-code` from the YAML — same envelope works.
  - Use the [pre-commit hook](../agent-harnesses.md#pre-commit-hook-git-users) as a harness-independent safety net.

If OpenCode adds a first-class shell-command hooks file later, mdvs will fill in `opencode/platform.toml`'s `[hooks]` section without other changes. Open an issue if you find one.

## Per-platform notes

- **Skill path**: `.opencode/skills/mdvs/SKILL.md` (native). OpenCode also reads `.claude/skills/` and `.agents/skills/` — you can symlink across if you want a single source of truth shared with another harness.
- **Project rules**: `AGENTS.md` at workspace root. OpenCode also reads `CLAUDE.md` as a Claude Code-compat fallback.

## Sources

- [OpenCode skills](https://opencode.ai/docs/skills/)
- [OpenCode agents](https://opencode.ai/docs/agents/)
- [OpenCode rules](https://opencode.ai/docs/rules/)
- [OpenCode-Hooks (community shell-bridge)](https://github.com/KristjanPikhof/OpenCode-Hooks)
