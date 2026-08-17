# Agent harnesses

mdvs ships agent integration in three pieces:

1. A **skill** (the [Agent Skills standard](https://agentskills.io)) — works in any harness that loads `.md` skills.
2. A **project-rules snippet** — works in any harness that reads `AGENTS.md` / `CLAUDE.md` / `.cursor/rules`.
3. A **PostToolUse hook** that calls `mdvs hook handle` — only verified end-to-end on Claude Code today.

Per-harness install steps in the left nav.

## How violations reach the agent (Claude Code)

When the agent edits a markdown file in your vault:

1. The harness's PostToolUse hook fires the configured `mdvs hook handle` command.
2. mdvs reads the tool-call payload, walks up to find `mdvs.toml`. If the edit happened outside any vault, the hook stays silent.
3. mdvs runs `check` on the vault. If the file is clean, the hook stays silent (no noise on the happy path).
4. If there are violations, mdvs writes a Claude-Code-shaped envelope JSON to stdout. The harness reads it and surfaces the markdown body to the agent through `additionalContext` and the pretty render to the user through `systemMessage`.
5. The agent sees the violation and reacts on its next turn — per the [schema-evolution loop](https://github.com/edochi/mdvs/blob/main/crates/mdvs/scaffolding/skill/SKILL.md): if it's a mistake, fix the file; if it's intentional (KB evolving), surface the deviation to the user and propose updating `mdvs.toml`.

A separate **search-nudge** hook fires after every Bash command that runs `grep` / `rg` / `find` / `ag` / `ack` / `fd` / `git grep`. If the agent's cwd is inside an mdvs vault, the hook surfaces a one-line tip suggesting `mdvs search`. Like validate, it's non-blocking — the agent decides whether to switch tools.

## Per-platform support

| Platform | Skill | Snippet | Hooks |
|---|---|---|---|
| [Claude Code](agent-harnesses/claude-code.md) | ✓ | ✓ | ✓ |
| [Codex](agent-harnesses/codex.md) | ✓ | ✓ | see [Codex hooks docs](https://developers.openai.com/codex/hooks) |
| [Cursor](agent-harnesses/cursor.md) | ✓ | ✓ | see [Cursor hooks docs](https://cursor.com/docs/hooks) |
| [OpenCode](agent-harnesses/opencode.md) | ✓ | ✓ | see [OpenCode docs](https://opencode.ai/docs/) |
| [Antigravity](agent-harnesses/antigravity.md) | ✓ | ✓ | see [Gemini CLI hooks docs](https://github.com/google-gemini/gemini-cli/tree/main/docs/hooks) |

## Pre-commit hook

A **pre-commit hook** is a script git runs locally before each `git commit` — if it exits non-zero, the commit is blocked. Running `mdvs check --no-update` there catches frontmatter violations before they reach the repo, **regardless of how the file was edited** — agent, IDE, or by hand.

That makes it the harness-independent safety net, and the recommended fallback for harnesses where the PostToolUse hook isn't wired up. It complements the hooks above rather than replacing them: the PostToolUse hook tells the agent mid-session, while the pre-commit hook is the backstop that catches whatever slipped through.

Full setup — both the [pre-commit framework](https://pre-commit.com/) and the plain `.git/hooks/pre-commit` script — is in the [pre-commit recipe](./pre-commit.md). For CI-side validation (catches violations even if a contributor skipped the local hook), see the [CI recipe](./ci.md).
