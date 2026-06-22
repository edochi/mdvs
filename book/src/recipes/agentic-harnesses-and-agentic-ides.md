# Agent Harnesses and Agentic IDEs

mdvs is a CLI, so integrating it with agent harnesses (Claude Code, Codex, OpenCode) and agentic IDEs (Cursor, Antigravity) is wiring rather than installing. This page covers the three integration points
- the bundled skill file
- the project-rules snippet
- two `PostToolUse` hooks

Claude Code config as the working reference, but the same pattern (with minor schema differences) applies to the others.

The goal is to establish a two-way feedback loop: the agent writes files, mdvs validates them on the spot, and the agent either fixes the violation or proposes a schema update if the deviation is intentional. The hook surfaces a **warning, not a block** (see [Schema evolution](#schema-evolution-warning-not-block) below).

## The skill file

mdvs ships a comprehensive `SKILL.md` (~350 lines) covering every command, the two-layer model, frontmatter formats, output shapes, and common workflows. The skill follows the [Agent Skills open standard](https://agentskills.io), originally released by Anthropic, which is now supported across the major agentic-coding harnesses.

The cross-harness path is `.agents/skills/<name>/SKILL.md`. This is the standard followed by most harnesses and agentic IDEs, like Codex ([source](https://developers.openai.com/codex/skills/)), OpenCode ([source](https://opencode.ai/docs/skills/)), Cursor ([source](https://cursor.com/docs/context/skills)), and Antigravity ([source](https://antigravity.google/docs/cli-plugins)).

```bash
mkdir -p .agents/skills/mdvs
mdvs skill > .agents/skills/mdvs/SKILL.md
```

Claude Code, on the other hand, reads only `.claude/skills/` ([source](https://code.claude.com/docs/en/skills)), so it gets its own line:

```bash
mkdir -p .claude/skills/mdvs
mdvs skill > .claude/skills/mdvs/SKILL.md
```

On systems where both harnesses are in use, a symlink (`ln -s ../../.agents/skills/mdvs .claude/skills/mdvs`) keeps a single source of truth.

## The project-rules snippet

For users who don't want a dedicated skill slot — or for the always-on context that supplements the lazily-loaded skill body — `mdvs skill --snippet` prints a short block to paste into the harness's project-rules file:

| Harness | File | Source |
|---|---|---|
| Claude Code | `CLAUDE.md` | [Skills docs](https://code.claude.com/docs/en/skills) |
| Codex | `AGENTS.md` (or `AGENTS.override.md`) | [AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md) |
| OpenCode | `AGENTS.md` | [Rules docs](https://opencode.ai/docs/rules/) |
| Cursor | `AGENTS.md` or `.cursor/rules/mdvs.mdc` | [Rules docs](https://cursor.com/docs/rules) |
| Antigravity | `AGENTS.md` | [Gemini CLI configuration](https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/configuration.md) |

```bash
mdvs skill --snippet >> AGENTS.md
```

The snippet tells the agent that this project has a markdown KB, that `mdvs search` should be preferred over `Grep` / `Glob` for KB lookups, and that frontmatter is validated by `mdvs check`. It also spells out the schema-evolution rule below.

## PostToolUse hook: validate on write

After every `Edit` or `Write` on a markdown file inside the KB, the hook runs `mdvs check --output markdown <file>` and surfaces the result back to the agent through the hook output channel. The output format is markdown, not JSON — JSON is for piping, markdown is what the agent reads best.

Claude Code config (`.claude/settings.json`, [hooks reference](https://code.claude.com/docs/en/hooks)):

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          {
            "type": "command",
            "command": "f=$(jq -r '.tool_input.file_path // empty'); test -n \"$f\" && mdvs check --output markdown \"$f\" 2>&1 || true"
          }
        ]
      }
    ]
  }
}
```

Claude Code passes a JSON object on stdin with fields `session_id`, `cwd`, `tool_name`, `tool_input` (containing `file_path` for `Edit` / `Write`), and `tool_output`. The `jq` one-liner extracts the path and feeds it to `mdvs check`. If the write violated the schema, the agent receives a markdown explanation: which file, which field, which rule, expected vs actual.

**The same shape works for other harnesses with minor adjustments:**

- **Codex** — same stdin-JSON contract; config goes in `.codex/hooks.json` (or the `[hooks]` table in `~/.codex/config.toml`). Event names match: `PreToolUse`, `PostToolUse`. [Hooks reference](https://developers.openai.com/codex/hooks).
- **Cursor** — same stdin-JSON contract; config goes in `.cursor/hooks.json`. Event names use camelCase: `postToolUse` instead of `PostToolUse`. [Hooks reference](https://cursor.com/docs/hooks).
- **OpenCode** — hooks are exposed through a TypeScript plugin API (`tool.execute.before`, `tool.execute.after`), not via a shell-command config file. To run a `mdvs check` shell command on every edit, either write an OpenCode plugin that shells out, or use a community package (e.g. [OpenCode-Hooks](https://github.com/KristjanPikhof/OpenCode-Hooks)) that adds a `hooks.yaml` shell-command layer on top. The validation contract is the same; only the wiring differs.

## Schema evolution: warning, not block

The validation hook warns rather than rejecting the agent's write. To keep the schema of the KB flexible (e.g., voluntary or sensible categories drift, fields shifting type, new conventions), the hooks do not block the agent from writing a file that deviates from the schema, instead they surface the deviation to the agent.

The skill file and the project-rules snippet describe what the agent should do when a warning fires. If the deviation is a mistake (a typo, the wrong type by accident, a dropped required field), the agent fixes the file. If the deviation is intentional (the KB is evolving, a category genuinely needs a fourth variant, a field is shifting type), the agent surfaces the deviation to the user and proposes updating `mdvs.toml` to absorb the change. The user decides whether to update the schema or revert the file.

## PostToolUse hook: nudge toward `mdvs search`

After `Grep` / `Glob` / similar search-style calls, emit an info-level reminder that `mdvs search` exists. Scope the hook to the KB directory so it doesn't fire on code-side greps:

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Grep|Glob",
        "hooks": [
          {
            "type": "command",
            "command": "p=$(jq -r '.tool_input.path // empty'); case \"$p\" in *kb/*) echo 'Tip: mdvs search runs hybrid semantic + full-text + SQL filtering over the KB. Often a better fit than Grep for content lookups.';; esac"
          }
        ]
      }
    ]
  }
}
```

Adjust `kb/` to your KB directory name. The hook only nudges; the agent decides whether to switch tools. For Codex and Cursor, swap the config path and (for Cursor) the camelCase event name; the stdin schema is the same.

## Pre-commit hook (git users)

If your project uses git, a `pre-commit` hook running `mdvs check` gives validation without any agent wiring. Git-dependent, but a useful safety net under any harness.

```yaml
# .pre-commit-config.yaml
repos:
  - repo: local
    hooks:
      - id: mdvs-check
        name: mdvs check
        entry: mdvs check --no-update
        language: system
        pass_filenames: false
```

For CI-side validation, see the [CI recipe](./ci.md).

## What's tested

At the time of writing, the validation hook has been verified end-to-end against Claude Code. For Codex, OpenCode, Cursor, and Antigravity, the configs above translate the same contract into each harness's documented schema; we have not yet run the full feedback loop on each. If you wire it up against a new harness and want the config snippet added here, [open an issue](https://github.com/edochi/mdvs/issues).

## Sources

- [Agent Skills open standard](https://agentskills.io)
- Claude Code: [skills](https://code.claude.com/docs/en/skills), [hooks](https://code.claude.com/docs/en/hooks)
- Codex: [skills](https://developers.openai.com/codex/skills/), [hooks](https://developers.openai.com/codex/hooks), [AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md)
- OpenCode: [skills](https://opencode.ai/docs/skills/), [agents](https://opencode.ai/docs/agents/), [rules](https://opencode.ai/docs/rules/)
- Cursor: [skills](https://cursor.com/docs/context/skills), [rules](https://cursor.com/docs/rules), [hooks](https://cursor.com/docs/hooks)
- Antigravity: [authoring skills codelab](https://codelabs.developers.google.com/getting-started-with-antigravity-skills), [Gemini CLI configuration (inherited)](https://github.com/google-gemini/gemini-cli/blob/main/docs/get-started/configuration.md)
