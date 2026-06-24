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

A **pre-commit hook** is a script git runs locally before each `git commit` — if it exits non-zero, the commit is blocked. The community [`pre-commit`](https://pre-commit.com/) tool manages hooks declaratively per-repo via a YAML config; mdvs plugs into it as a one-line entry.

Running `mdvs check` as a pre-commit hook catches frontmatter violations before they reach the repo, **regardless of how the file was edited** — agent, IDE, or by hand. It's the simplest harness-independent safety net, and the recommended fallback for harnesses where the post-edit hook isn't wired up.

### Install

To install the `pre-commit` tool on your machine check the docs at this [link](https://pre-commit.com/#install).

It's also possible to install `pre-commit` using [`uv`](https://docs.astral.sh/uv/):

```bash
uv tool install pre-commit
```

### Configure

In your mdvs vault, create `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: local
    hooks:
      - id: mdvs-check
        name: mdvs check
        entry: mdvs check --no-update
        language: system
        pass_filenames: false
```

Activate the hook in this repo (writes `.git/hooks/pre-commit`):

```bash
pre-commit install
```

That's it. The next `git commit` runs `mdvs check`; if there are violations the commit aborts and the violation report is printed. To run the check manually without committing:

```bash
pre-commit run --all-files
```

### Notes

- **Works with any install method.** `language: system` just runs the `mdvs` already on your PATH — it doesn't matter whether you installed via `cargo install mdvs`, the release shell installer, Homebrew, or a manually-placed binary. The only requirement is that `mdvs` is invocable from git's environment.
- **PATH gotcha for GUI git clients.** git pre-commit hooks fire under git's environment, which isn't always the same as your interactive shell's PATH. If `mdvs` lives in `~/.cargo/bin/` and you commit from a GUI client that doesn't inherit your shell PATH, the hook fails with `mdvs: command not found`. Either commit from the terminal, or use the absolute path in `entry:` (`entry: /Users/you/.cargo/bin/mdvs check --no-update`).
- **Version-pinned alternative.** To have `pre-commit` fetch `mdvs` into its own isolated environment (slower per-repo install, but reproducible across machines and CI), swap to `language: rust` and `additional_dependencies: ["mdvs"]`.
- `--no-update` tells `mdvs check` not to auto-update `mdvs.toml` from inferred new fields. The hook validates against the committed schema; schema evolution stays an explicit user action.
- `pass_filenames: false` because `mdvs check` runs against the whole vault, not file-by-file. The same validation pass covers every staged change in one shot.

For CI-side validation (catches violations even if a contributor skipped the local hook), see the [CI recipe](./ci.md).
