# Claude Code

## Install

Three commands. Run each in your project root:

```bash
mkdir -p .claude/skills/mdvs
mdvs scaffold skill > .claude/skills/mdvs/SKILL.md
mdvs scaffold snippet --platform claude-code >> CLAUDE.md
mdvs scaffold hook --platform claude-code >> .claude/settings.json   # merge into existing hooks
```

The last command emits a JSON snippet — if `.claude/settings.json` already exists with other settings, **merge by hand** instead of appending blindly: the `hooks.PostToolUse` array should be unioned with anything you already have. mdvs's emitted snippet self-documents the merge target in a `_comment` field at the top.

## What you get

- **Skill**: agent learns when to call which mdvs command, how to interpret violations, and the schema-evolution loop. Loaded on session start; activated by description-match or directly via `/mdvs`.
- **Snippet**: always-on `CLAUDE.md` block telling the agent to prefer `mdvs search` over `Grep` for KB lookups.
- **Validate hook**: after every `Edit` / `Write` / `MultiEdit` on a markdown file inside an mdvs vault, `mdvs hook handle` runs `check` and surfaces violations through `additionalContext` (agent-visible) and `systemMessage` (user-visible, capped at 15 lines). Hook always exits 0 — never blocks.
- **Search-nudge hook**: after every `Bash` command that runs `grep` / `rg` / `find` / etc., if the agent's cwd is in an mdvs vault, surfaces a one-line tip pointing at `mdvs search`.

## Per-platform notes

- **Skill path**: `.claude/skills/mdvs/SKILL.md` (Claude Code reads only from `.claude/skills/`, not the cross-harness `.agents/skills/`).
- **Project rules**: `CLAUDE.md` at workspace root.
- **Hook envelope**: Claude Code's `hookSpecificOutput.additionalContext` + `systemMessage` shape. PascalCase event name (`PostToolUse`).
- **mdvs on PATH**: the hook command is `mdvs hook handle --platform claude-code --kind <kind>`.

## Sources

- [Claude Code skills](https://code.claude.com/docs/en/skills)
- [Claude Code hooks](https://code.claude.com/docs/en/hooks)
