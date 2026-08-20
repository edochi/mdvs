# CI

`mdvs check` exits with code 1 when any file violates the schema, so it slots
straight into a CI pipeline as a frontmatter linter. This page covers the GitHub
Actions case, but the same shape works on GitLab CI, CircleCI, or any runner
that can install a binary and run a command.

## Minimal GitHub Actions workflow

```yaml
# .github/workflows/check-frontmatter.yml
name: Frontmatter check

on:
  push:
    branches: [main]
  pull_request:

jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install mdvs
        env:
          MDVS_VERSION: vX.Y.Z   # pin to a specific release — see below
        run: |
          curl --proto '=https' --tlsv1.2 -LsSf \
            "https://github.com/edochi/mdvs/releases/download/${MDVS_VERSION}/mdvs-installer.sh" | sh
          echo "$HOME/.cargo/bin" >> $GITHUB_PATH

      - name: Validate frontmatter
        run: mdvs check --no-update
```

Replace `vX.Y.Z` with a real release tag (see the
[releases page](https://github.com/edochi/mdvs/releases)). This adds a check
that runs on every PR and every push to `main`. If a contributor introduces a
file with a wrong type, missing required field, disallowed field, or
unrepresentable frontmatter, the job fails and the PR is blocked until it's
fixed.

## Pin the mdvs version

The installer URL above pulls a **specific release tag**. GitHub also exposes a
`releases/latest/download/...` URL that always redirects to the newest release —
convenient for casual use, and that's what the
[README install snippet](https://github.com/edochi/mdvs#install) uses — but in
CI you want **reproducibility**. Pinning a specific tag means a green check
today still passes (or still fails the same way) tomorrow, regardless of what
mdvs ships in the meantime.

Bump the pinned version when you're ready to adopt new validation behavior. The
mdvs [release notes](https://github.com/edochi/mdvs/releases) call out anything
that affects validation output.

## `--no-update` for deterministic CI

The `--no-update` flag (or `[check].auto_update = false` in `mdvs.toml`) tells
`check` to validate against the committed schema instead of re-running inference
first. This matters in CI:

- **With auto-update on:** `check` re-infers the schema before validating and
  **rewrites `mdvs.toml` on disk**, absorbing any new frontmatter field. On a
  runner that rewrite is thrown away with the checkout, but the run validated
  against a schema that differs from the committed one, and the new field is
  never mentioned.
- **With `--no-update`:** `mdvs.toml` is left untouched and the run validates
  against exactly what's committed. A new field is reported under a **New
  fields** heading in the output.

Re-inference only ever _adds_ fields — it does not widen an existing field's
type or relax its constraints. A value that breaks a declared `categories` list,
or a field with the wrong type, fails in both modes.

In practice: in CI, **always** use `--no-update`. You validate against the
committed schema, nothing is rewritten mid-run, and additions are visible in the
log.

### `check` does not fail on undeclared fields

Worth being explicit, because it is easy to assume otherwise: a frontmatter
field that appears in **no** `[[fields.field]]` entry is **not** a violation, in
either mode. `--no-update` surfaces it as informational output and the command
still **exits 0**.

```
Checked 2 files — no violations, 1 new field(s)
```

Validation iterates the fields declared in `mdvs.toml`, so a key it has never
seen is reported, not rejected. `Disallowed` means something narrower: a
_declared_ field appearing at a path outside its `allowed` globs.

If you want new fields to break the build today, gate on the JSON output — it
carries a `new_fields` array:

```bash
mdvs check --no-update --output json | jq -e '.new_fields | length == 0'
```

`jq -e` exits non-zero when the expression is false, so the step fails as soon
as an undeclared field shows up. Run it _in addition to_
`mdvs check --no-update`, which still owns the real violations. A schema-level
"freeze this directory" option does not exist yet.

## Caching the install

The installer step downloads a small binary (~6 MB on Linux) and finishes in
well under a second. There's usually no point caching it. If you want to avoid
the network call entirely on every run, use `actions/cache` keyed on the mdvs
version string, or commit a vendored binary into the repo and skip the install
step.

## What `check` does (and doesn't)

`mdvs check` covers frontmatter validation only:

- ✓ Wrong types (a `Boolean` field with a string value)
- ✓ Missing required fields per directory
- ✓ Disallowed fields (a declared field appearing outside its `allowed` paths)
- ✓ Null violations
- ✓ Category, length, range, and regex constraint violations
- ✓ Frontmatter that can't be parsed at all (broken YAML, broken TOML, broken
  JSON)

It does **not** flag a field that appears in no `[[fields.field]]` entry — see
[above](#check-does-not-fail-on-undeclared-fields). It also does **not** check
spelling, link validity, markdown style, or anything in the body content. Pair
it with a markdown linter (markdownlint, vale) for those concerns. They run
independently and have no conflict — `mdvs check` and a body-content linter
cover orthogonal parts of the file.

## Other CI systems

The shape translates directly:

- **GitLab CI:** the same two-step install-then-run pattern in `.gitlab-ci.yml`.
  Use the install script under `before_script:` and run `mdvs check --no-update`
  in the job.
- **CircleCI:** an `orb` or a custom step that installs the binary and invokes
  the check.
- **Pre-commit hook:** `mdvs check --no-update` as a hook entry in
  `.pre-commit-config.yaml` runs the check locally on every commit, catching
  issues before they reach CI. See the dedicated
  [pre-commit recipe](./pre-commit.md) for both the framework and plain-git-hook
  setups.

The contract is always the same: install `mdvs`, run `mdvs check --no-update`,
fail on non-zero exit.
