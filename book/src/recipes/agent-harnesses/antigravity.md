# Antigravity

> **Skill and snippet only — no hooks.** Antigravity CLI's hook surface is undocumented post-rebrand (it inherits Gemini CLI's hooks reference but Google hasn't published the exact post-rebrand schema). `mdvs scaffold hook --platform antigravity` refuses with a pointer; skill and snippet still install normally and cover the most common workflow (agent reads the skill body, agent reads `AGENTS.md` on every turn, agent uses `mdvs search` proactively for KB lookups).

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
- **Hooks**: when Google publishes a documented PostToolUse-style schema for Antigravity, mdvs will add the `[hooks]` section to `antigravity/platform.toml` without other changes needed. If you find an authoritative source for the schema before then, [open an issue](https://github.com/edochi/mdvs/issues) with the link.

## Sources

- [Authoring Google Antigravity Skills (Codelab)](https://codelabs.developers.google.com/getting-started-with-antigravity-skills)
- [Gemini CLI configuration (inherited reference)](https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/configuration.md)
