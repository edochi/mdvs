# OpenCode

> **Skill and snippet only — no hooks.** OpenCode's hook surface is the TypeScript plugin API (`tool.execute.before` / `tool.execute.after`), not a shell-command config file. mdvs's `scaffold hook` command refuses for OpenCode with a pointer at this page. Skill and snippet still install normally.

For the design intent behind the integration and the runtime story, see the [Agent harnesses overview](../agent-harnesses.md). For copy-paste install steps, read on.

## Install

```bash
mkdir -p .opencode/skills/mdvs
mdvs scaffold skill > .opencode/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform opencode >> AGENTS.md
```

OpenCode reads skills from `.opencode/skills/` natively, plus `.claude/skills/` and `.agents/skills/` for cross-harness compatibility. `mdvs scaffold skill --platform opencode` writes to `.opencode/skills/` (the native path) by default.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded by OpenCode on session start.
- **Snippet**: always-on `AGENTS.md` block telling the agent to prefer `mdvs search` over `Grep`.
- **Hooks**: not installed by `mdvs scaffold hook`, but a [bridge plugin](#workaround-typescript-bridge-plugin) is available — see below.

If OpenCode adds a first-class shell-command hooks file later (tracked at [opencode#12472](https://github.com/anomalyco/opencode/issues/12472)), mdvs will fill in `opencode/platform.toml`'s `[hooks]` section without other changes. Open an issue if you find one shipped.

## Workaround: TypeScript bridge plugin

While [opencode#12472](https://github.com/anomalyco/opencode/issues/12472) is still open, the path to post-edit validation feedback in OpenCode is a small TypeScript plugin that calls `mdvs hook handle` from within OpenCode's plugin event API. The plugin is ~80 lines, has no dependencies beyond what OpenCode already provides, and forwards the violation report into the agent's context.

A working reference implementation lives in the mdvs repository at [`crates/mdvs/examples/opencode-plugin/mdvs-hooks.ts`](https://github.com/edochi/mdvs/blob/main/crates/mdvs/examples/opencode-plugin/mdvs-hooks.ts). It targets the `tool.execute.after` event and invokes:

```
mdvs hook handle --platform claude-code --kind <kind>
```

The Claude Code envelope is parseable text that OpenCode's plugin forwards via `client.session.prompt()`, which becomes a tagged `[mdvs hook]` message in the agent's next turn. Same walk-up-silent-outside-a-vault behaviour as the native hook in other harnesses.

To install, copy the plugin file into `.opencode/plugin/mdvs-hooks.ts` at your project root (or `~/.config/opencode/plugin/` for user-scope). Requires `mdvs` on the OpenCode process's PATH.

Two limitations vs the native hooks Claude Code/Codex/Cursor get:

- Injection is via `client.session.prompt()`, which creates a new user-style turn instead of silently appending to the model's context. The `[mdvs hook]` tag flags it as a hook-generated message; the agent's skill can be taught to recognise the prefix.
- No `systemMessage`-equivalent — the violation reaches the agent but not the user UI directly.

Both improvements depend on what OpenCode's plugin API eventually exposes. As a fallback, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook-git-users) is a harness-independent safety net that runs `mdvs check` on every commit.

## Per-platform notes

- **Skill path**: `.opencode/skills/mdvs/SKILL.md` (native). OpenCode also reads `.claude/skills/` and `.agents/skills/` — you can symlink across if you want a single source of truth shared with another harness.
- **Project rules**: `AGENTS.md` at workspace root. OpenCode also reads `CLAUDE.md` as a Claude Code-compat fallback.

## Sources

- [OpenCode skills](https://opencode.ai/docs/skills/)
- [OpenCode agents](https://opencode.ai/docs/agents/)
- [OpenCode rules](https://opencode.ai/docs/rules/)
- [OpenCode-Hooks (community shell-bridge)](https://github.com/KristjanPikhof/OpenCode-Hooks)
