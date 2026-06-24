# Antigravity

## Install

```bash
mkdir -p .agents/skills/mdvs
mdvs scaffold skill > .agents/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform antigravity >> AGENTS.md
```

Antigravity reads skills from `.agents/skills/<name>/SKILL.md` (the cross-harness Agent Skills convention — same path Codex uses) and reads project rules from `AGENTS.md` at the workspace root.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded by Antigravity on session start.
- **Snippet**: always-on project-rules block telling the agent to prefer `mdvs search` over `Grep` for KB lookups.

## Hooks

mdvs doesn't ship a verified Antigravity hook config. Antigravity inherits parts of its configuration sources from Gemini CLI, so the hooks system should be compatible with what's documented at the [Gemini CLI hooks reference](https://github.com/google-gemini/gemini-cli/tree/main/docs/hooks).

As a harness-independent fallback, the [pre-commit hook](../agent-harnesses.md#pre-commit-hook) runs `mdvs check` on every commit.

## Per-platform notes

- **Skill install path**: `.agents/skills/mdvs/SKILL.md`. Project-scoped path; the agent picks it up when working in the directory.
- **AGENTS.md**: documented as the post-rebrand convention. Legacy Gemini CLI sessions also recognized `GEMINI.md`; both still appear in flux.

## Sources

- [Authoring Google Antigravity Skills (Codelab)](https://codelabs.developers.google.com/getting-started-with-antigravity-skills)
- [Gemini CLI configuration (inherited reference)](https://github.com/google-gemini/gemini-cli/tree/main/docs/hooks)
