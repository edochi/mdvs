# Pre-commit hook

`mdvs check` exits non-zero when any file violates the schema, so it works as a
git pre-commit hook: run it before each commit and frontmatter mistakes never
reach the repo. This is the local counterpart to the [CI recipe](./ci.md) — same
command, same contract, one step earlier in the loop. Catching a violation at
commit time is faster than waiting for a CI job to fail on the pushed branch.

There are two ways to wire it up: the
[pre-commit framework](https://pre-commit.com) (if you already use it) or a
plain git hook script (zero dependencies). Both run the same command.

## Use `--no-update`

Every setup below runs `mdvs check --no-update`. The flag validates against the
committed `mdvs.toml` instead of re-running inference first.

This matters more in a hook than in CI. Without it, `check` **rewrites
`mdvs.toml` on disk** as it runs — so a commit that adds a new frontmatter field
silently edits your schema file mid-commit, leaving a modified `mdvs.toml` in
your working tree that isn't part of what you staged. With it, the file is left
alone and the new field is reported instead.

Note what `--no-update` does _not_ do: an undeclared field is **not** a
violation, and the hook still exits 0, so it won't block the commit. See
[`check` does not fail on undeclared fields](./ci.md#check-does-not-fail-on-undeclared-fields)
for what does and doesn't gate. The
[CI recipe covers the flag in full](./ci.md#--no-update-for-deterministic-ci);
the short version is: **always use `--no-update` in a hook.**

## pre-commit framework

To install the `pre-commit` tool itself, see
[its install docs](https://pre-commit.com/#install), or use
[`uv`](https://docs.astral.sh/uv/):

```bash
uv tool install pre-commit
```

Then add mdvs to your vault's `.pre-commit-config.yaml`:

```yaml
repos:
  - repo: https://github.com/edochi/mdvs
    rev: vX.Y.Z          # pin to a released tag — see the releases page
    hooks:
      - id: mdvs-check
```

Replace `vX.Y.Z` with a real
[release tag](https://github.com/edochi/mdvs/releases). This references the
[`.pre-commit-hooks.yaml`](https://github.com/edochi/mdvs/blob/main/.pre-commit-hooks.yaml)
shipped in the mdvs repo, which declares the hook as:

```yaml
- id: mdvs-check
  name: mdvs check
  entry: mdvs check --no-update
  language: system
  types: [markdown]
  pass_filenames: false
```

`language: system` means the hook expects `mdvs` to already be on `PATH` —
pre-commit will not build it from source. Install mdvs once (see the
[README install snippet](https://github.com/edochi/mdvs#install)), then activate
the hook:

```bash
pre-commit install
```

The next `git commit` runs `mdvs check`; if there are violations the commit
aborts and the violation report is printed. To run the check manually without
committing:

```bash
pre-commit run --all-files
```

`pass_filenames: false` is deliberate: `mdvs check` validates the whole vault,
not a file list, because rules like "required field per directory" need the full
tree. `types: [markdown]` scopes the trigger so the hook only fires when a
commit touches markdown.

### Notes

- **Works with any install method.** `language: system` just runs the `mdvs`
  already on your PATH — it doesn't matter whether you installed via
  `cargo install mdvs`, the release shell installer, Homebrew, or a
  manually-placed binary. The only requirement is that `mdvs` is invocable from
  git's environment.
- **PATH gotcha for GUI git clients.** git hooks fire under git's environment,
  which isn't always the same as your interactive shell's PATH. If `mdvs` lives
  in `~/.cargo/bin/` and you commit from a GUI client that doesn't inherit your
  shell PATH, the hook fails with `mdvs: command not found`. Either commit from
  the terminal, or use an absolute path
  (`entry: /Users/you/.cargo/bin/mdvs check --no-update`). The same applies to
  the plain git hook below.
- **Version-pinned alternative.** To have `pre-commit` fetch `mdvs` into its own
  isolated environment (slower per-repo install, but reproducible across
  machines and CI), define the hook inline with `repo: local`, `language: rust`,
  and `additional_dependencies: ["mdvs"]` instead of referencing this repo.

## Plain git hook

No framework, no dependencies — just a script git runs before each commit. Write
it to `.git/hooks/pre-commit`:

```bash
#!/bin/sh
mdvs check --no-update || exit 1
```

```bash
# from the repo root:
cat > .git/hooks/pre-commit <<'EOF'
#!/bin/sh
mdvs check --no-update || exit 1
EOF
chmod +x .git/hooks/pre-commit
```

This is the whole thing. `mdvs check --no-update` exits 1 on a schema violation
and 2 on an internal error; either non-zero status aborts the commit. On a clean
vault it exits 0 and the commit proceeds.

The one caveat: `.git/hooks/` is not version-controlled, so this script lives
only in your local clone — each contributor sets it up themselves. If you want
the hook shared across a team automatically, use the pre-commit framework above
(its config _is_ committed) or point `core.hooksPath` at a tracked directory:

```bash
git config core.hooksPath .githooks   # commit your hook script under .githooks/
```

## Scope

The hook runs `mdvs check` over the whole vault, not just the files staged for
the commit. That's the same trade-off the [CI recipe](./ci.md) makes:
whole-vault is simpler and catches cross-file violations (a missing required
field, a duplicate that only conflicts in aggregate), at the cost of also
re-validating files the commit didn't touch.

In practice that cost is negligible — validation is a frontmatter pass, not an
embedding pass, and runs in milliseconds even on vaults of a few thousand files.
Validating only the staged files would need a per-file validation mode, which
mdvs does not expose: `check` takes a vault path, not a file list. If you have a
vault where the whole-vault pass is actually too slow, open an issue.

Like in CI, `mdvs check` covers frontmatter only — types, required fields,
disallowed fields, nulls, constraints, and unparseable frontmatter. It does not
check body content, spelling, or links. Pair it with a markdown linter for
those; they run independently.
