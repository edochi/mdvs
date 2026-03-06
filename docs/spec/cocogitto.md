# Cocogitto

Cocogitto (`cog`) enforces [Conventional Commits](https://www.conventionalcommits.org/) and automates changelog generation. Configuration lives in `cog.toml` at the repo root.

## Install

```bash
cargo install cocogitto
```

This installs the `cog` binary.

## Setup (after cloning)

Install the commit-msg git hook so non-conventional commits are rejected locally:

```bash
cog install-hook commit-msg
```

This reads the hook definition from `cog.toml` and creates `.git/hooks/commit-msg`. The hook runs `cog verify` on every commit message.

## Conventional Commit Format

```
<type>[optional scope]: <description>

[optional body]

[optional footer(s)]
```

### Types

| Type | When to use |
|------|-------------|
| `feat` | New feature or capability |
| `fix` | Bug fix |
| `refactor` | Code change that neither fixes a bug nor adds a feature |
| `docs` | Documentation only |
| `test` | Adding or updating tests |
| `chore` | Build, config, tooling, dependencies |
| `ci` | CI/CD changes |
| `perf` | Performance improvement |
| `style` | Formatting, whitespace (no code change) |

### Scope (optional)

Scope narrows the change area. Examples: `feat(search): ...`, `fix(build): ...`, `chore(deps): ...`.

### Breaking changes

Append `!` after the type/scope:

```
feat!: remove --human flag in favor of --text
refactor(search)!: change result struct shape
```

### Examples

```
feat: add enum constraints on string fields
fix(build): track removed chunk counts correctly
docs: add cocogitto setup guide
chore(deps): bump datafusion to 53
refactor: extract validate() from check command
ci: add GitHub Actions CI workflow
test(init): add verbose output tests
feat!: rename --where-clause to --where
```

## Changelog Generation

Cocogitto generates `CHANGELOG.md` from conventional commits when bumping versions:

```bash
cog bump --auto    # determine version from commit types (feat→minor, fix→patch)
cog bump --major   # force major bump
cog bump --minor   # force minor bump
cog bump --patch   # force patch bump
```

The `[changelog]` section in `cog.toml` configures GitHub links for commits and PRs.

## Verify Commits

```bash
cog check          # validate all commits from initial or last tag
cog verify "feat: something"   # validate a single message
```

## Configuration Reference

See `cog.toml` in the repo root. Key settings:

| Key | Value | Purpose |
|-----|-------|---------|
| `tag_prefix` | `"v"` | Tags are `v0.1.0`, `v0.2.0`, etc. |
| `from_latest_tag` | `false` | Check all commits, not just since last tag |
| `ignore_merge_commits` | `false` | Merge commits must also be conventional |
| `[changelog]` | GitHub remote info | Enables commit/PR links in generated changelog |
| `[git_hooks.commit-msg]` | Hook script | Runs `cog verify` on every commit |
