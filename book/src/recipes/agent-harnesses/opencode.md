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

## Reference: TypeScript bridge plugin (untested — please help)

> **Did not fire in initial smoke test.** The plugin below is a *reference implementation* written against OpenCode's documented plugin API ([opencode.ai/docs/skills/](https://opencode.ai/docs/skills/), [opencode#12472](https://github.com/anomalyco/opencode/issues/12472)). In a live smoke test against a real vault, the plugin was not observed firing — the event signature, plugin loader path, exec environment, or prompt-injection call may all need adjusting. Treat the bridge as a starting point for diagnosis, not a working tool. If you debug it into a working state, please [open a PR](https://github.com/edochi/mdvs/issues).

While [opencode#12472](https://github.com/anomalyco/opencode/issues/12472) is open, the architectural path to post-edit validation feedback in OpenCode is a small TypeScript plugin that calls `mdvs hook handle` from OpenCode's plugin event API. The plugin is ~80 lines and forwards the violation report into the agent's context. The reference implementation lives in the mdvs repository at [`crates/mdvs/examples/opencode-plugin/mdvs-hooks.ts`](https://github.com/edochi/mdvs/blob/main/crates/mdvs/examples/opencode-plugin/mdvs-hooks.ts). It targets the `tool.execute.after` event and invokes:

```
mdvs hook handle --platform claude-code --kind <kind>
```

The intended flow: the Claude Code envelope is parseable text that OpenCode's plugin would forward via `client.session.prompt()`, becoming a tagged `[mdvs hook]` message in the agent's next turn.

To try it, copy the plugin file into `.opencode/plugin/mdvs-hooks.ts` at your project root (or `~/.config/opencode/plugin/` for user-scope). Requires `mdvs` on the OpenCode process's PATH. Then watch OpenCode's stderr / `~/.local/share/opencode/log/` for any sign of the plugin loading or the event firing.

Two design limitations vs native shell hooks (assuming the bridge works at all):

- Injection would be via `client.session.prompt()`, which creates a new user-style turn instead of silently appending to the model's context. The `[mdvs hook]` tag flags it as a hook-generated message; the agent's skill can be taught to recognise the prefix.
- No `systemMessage`-equivalent — the violation would reach the agent but not the user UI directly.

As a fallback that works regardless of the bridge's state, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook-git-users) is a harness-independent safety net that runs `mdvs check` on every commit.

## Per-platform notes

- **Skill path**: `.opencode/skills/mdvs/SKILL.md` (native). OpenCode also reads `.claude/skills/` and `.agents/skills/` — you can symlink across if you want a single source of truth shared with another harness.
- **Project rules**: `AGENTS.md` at workspace root. OpenCode also reads `CLAUDE.md` as a Claude Code-compat fallback.

## Sources

- [OpenCode skills](https://opencode.ai/docs/skills/)
- [OpenCode agents](https://opencode.ai/docs/agents/)
- [OpenCode rules](https://opencode.ai/docs/rules/)
- [OpenCode-Hooks (community shell-bridge)](https://github.com/KristjanPikhof/OpenCode-Hooks)
