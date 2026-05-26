# CI

`mdvs check` exits with code 1 when any file violates the schema, so it slots straight into a CI pipeline as a frontmatter linter. This page covers the GitHub Actions case, but the same shape works on GitLab CI, CircleCI, or any runner that can install a binary and run a command.

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

Replace `vX.Y.Z` with a real release tag (see the [releases page](https://github.com/edochi/mdvs/releases)). This adds a check that runs on every PR and every push to `main`. If a contributor introduces a file with a wrong type, missing required field, disallowed field, or unrepresentable frontmatter, the job fails and the PR is blocked until it's fixed.

## Pin the mdvs version

The installer URL above pulls a **specific release tag**. GitHub also exposes a `releases/latest/download/...` URL that always redirects to the newest release — convenient for casual use, and that's what the [README install snippet](https://github.com/edochi/mdvs#install) uses — but in CI you want **reproducibility**. Pinning a specific tag means a green check today still passes (or still fails the same way) tomorrow, regardless of what mdvs ships in the meantime.

Bump the pinned version when you're ready to adopt new validation behavior. The mdvs [release notes](https://github.com/edochi/mdvs/releases) call out anything that affects validation output.

## `--no-update` for deterministic CI

The `--no-update` flag (or `[check].auto_update = false` in `mdvs.toml`) tells `check` to validate strictly against the committed schema instead of re-running inference first. This matters in CI:

- **With auto-update on:** a PR that adds a new frontmatter field will pass because `check` re-infers the schema and silently includes the new field. The unintended addition slips through.
- **With `--no-update`:** the same PR fails with a `Disallowed` violation because the new field isn't in the committed `mdvs.toml`. The contributor has to either remove the field, add it to the schema deliberately, or add it to the `ignore` list — all of which surface the decision.

In practice this means: in CI, **always** use `--no-update`. Run `mdvs update` locally when you want to add new fields, commit the resulting `mdvs.toml`, and the CI run will then pass.

## Caching the install

The installer step downloads a small binary (~6 MB on Linux) and finishes in well under a second. There's usually no point caching it. If you want to avoid the network call entirely on every run, use `actions/cache` keyed on the mdvs version string, or commit a vendored binary into the repo and skip the install step.

## What `check` does (and doesn't)

`mdvs check` covers frontmatter validation only:

- ✓ Wrong types (a `Boolean` field with a string value)
- ✓ Missing required fields per directory
- ✓ Disallowed fields (anything not in `mdvs.toml` and not in `ignore`)
- ✓ Null violations
- ✓ Category, length, range, and regex constraint violations
- ✓ Frontmatter that can't be parsed at all (broken YAML, broken TOML, broken JSON)

It does **not** check spelling, link validity, markdown style, or anything in the body content. Pair it with a markdown linter (markdownlint, vale) for those concerns. They run independently and have no conflict — `mdvs check` and a body-content linter cover orthogonal parts of the file.

## Other CI systems

The shape translates directly:

- **GitLab CI:** the same two-step install-then-run pattern in `.gitlab-ci.yml`. Use the install script under `before_script:` and run `mdvs check --no-update` in the job.
- **CircleCI:** an `orb` or a custom step that installs the binary and invokes the check.
- **Pre-commit hook:** `mdvs check --no-update` as a hook entry in `.pre-commit-config.yaml` runs the check locally on every commit, catching issues before they reach CI.

The contract is always the same: install `mdvs`, run `mdvs check --no-update`, fail on non-zero exit.
