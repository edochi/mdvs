# Antigravity

> **Skill and snippet only — no project-level hooks.** Antigravity CLI's hook system is user-scope only: per [the changelog](https://github.com/google-antigravity/antigravity-cli/blob/main/CHANGELOG.md), the `/hooks` command writes to the shared `~/.gemini/config/hooks.json` file, and no per-project hooks config path is documented. `mdvs scaffold hook --platform antigravity` therefore refuses with a pointer here — skill and snippet still install normally and cover the most common workflow (agent reads the skill body, agent reads `AGENTS.md` on every turn, agent uses `mdvs search` proactively for KB lookups).

For the design intent behind the integration and the runtime story, see the [Agent harnesses overview](../agent-harnesses.md). For copy-paste install steps, read on.

## Install

```bash
mdvs scaffold skill > .agents/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform antigravity >> AGENTS.md
```

Antigravity reads skills from `.agents/skills/<name>/SKILL.md` (the cross-harness Agent Skills convention — same path Codex uses) and reads project rules from `AGENTS.md` at the workspace root.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded by Antigravity on session start.
- **Snippet**: always-on project-rules block telling the agent to prefer `mdvs search` over `Grep` for KB lookups and to react to validation warnings (when they reach it via some other channel — see "Per-platform notes" below).
- **Hooks**: not installed. The agent doesn't get automatic feedback after Edit/Write. You can still run `mdvs check` manually (or as a pre-commit hook) to catch violations before they merge.

## Per-platform notes

- **Skill install path**: `.agents/skills/mdvs/SKILL.md` (per [Google's authoring codelab](https://codelabs.developers.google.com/getting-started-with-antigravity-skills)). Project-scoped path; the agent picks it up when working in the directory.
- **AGENTS.md**: documented as the post-rebrand convention. Legacy Gemini CLI sessions also recognized `GEMINI.md`; both still appear in flux.
- **Project-level hooks**: not supported upstream — Antigravity's `/hooks` command writes to a shared user-scope file (`~/.gemini/config/hooks.json`), with no per-project override path documented as of mid-2026. If a per-project config path lands later, mdvs will add the `[hooks]` section to `antigravity/platform.toml` without other code changes needed. If you find one in the docs before then, [open an issue](https://github.com/edochi/mdvs/issues) with the link.

## Workaround: user-level hooks

If post-edit validation feedback in Antigravity matters enough to give up per-project scoping, the user-level hooks file CAN be edited by hand to call `mdvs hook handle`. The hook's built-in walk-up logic means it stays silent outside an mdvs vault, so the user-level install is safe even in projects that aren't mdvs vaults — it just no-ops there.

Open `~/.gemini/config/hooks.json` (create it if absent) and add a `PostToolUse` entry that runs:

```
mdvs hook handle --platform claude-code --kind validate
```

(The `claude-code` platform's envelope is forwarded by `mdvs hook handle`; check Antigravity's hooks reference for the exact JSON shape Antigravity expects in its hooks file, and adapt the entry accordingly.)

This is an unofficial path and unverified against a live Antigravity session — install at your own risk.

## Sources

- [Authoring Google Antigravity Skills (Codelab)](https://codelabs.developers.google.com/getting-started-with-antigravity-skills)
- [Gemini CLI configuration (inherited reference)](https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/configuration.md)
