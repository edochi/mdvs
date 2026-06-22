# Codex

> **Schema-correct but no live smoke test yet.** The install commands produce output that matches the Codex hooks reference (envelope shape, event names, config file path), and the runtime envelope template is structurally correct per [the docs](https://developers.openai.com/codex/hooks). The full feedback loop — bogus-frontmatter edit triggering the hook in a live Codex session — hasn't been verified end-to-end. If you wire mdvs into Codex and hit a wiring bug, please [open an issue](https://github.com/edochi/mdvs/issues).

For the design intent behind the integration and the runtime story, see the [Agent harnesses overview](../agent-harnesses.md). For copy-paste install steps, read on.

## Install

```bash
mkdir -p .agents/skills/mdvs
mdvs scaffold skill > .agents/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform codex >> AGENTS.md
mdvs scaffold hook --platform codex >> .codex/hooks.json   # merge into existing hooks
```

If `.codex/hooks.json` already exists, **merge by hand** — the `hooks.PostToolUse` array should union with anything you already have.

Codex also accepts hooks declared via the `[hooks]` table in `~/.codex/config.toml`. mdvs emits the JSON form by default for consistency with the other harnesses; you can paste the equivalent TOML into your `config.toml` if you prefer.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded from `.agents/skills/mdvs/SKILL.md` (the cross-harness Agent Skills convention).
- **Snippet**: always-on `AGENTS.md` block telling the agent to prefer `mdvs search` over `Grep`.
- **Validate hook**: after every `Edit` / `Write` / `MultiEdit` on a markdown file inside an mdvs vault, `mdvs hook handle` runs `check` and surfaces violations through `additionalContext` and `systemMessage`. Always exits 0.
- **Search-nudge hook**: same as the validate hook but matching Bash search-tool invocations inside a vault.

## Per-platform notes

- **Skill path**: `.agents/skills/mdvs/SKILL.md` (Codex's canonical path per [their skills docs](https://developers.openai.com/codex/skills/) — same path Cursor and Antigravity also honor).
- **Project rules**: `AGENTS.md` at workspace root. `AGENTS.override.md` takes precedence if present.
- **Hook envelope**: same shape as Claude Code (`hookSpecificOutput.additionalContext` + `systemMessage`), PascalCase event name (`PostToolUse`).
- **mdvs on PATH**: `mdvs` must be available to the hook subprocess. `cargo install --path crates/mdvs` is the simplest install.

## Sources

- [Codex skills](https://developers.openai.com/codex/skills/)
- [Codex hooks](https://developers.openai.com/codex/hooks)
- [Codex AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md)
