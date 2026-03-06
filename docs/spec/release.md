# Release Process

## Overview

Releases are semi-automated. You decide when to release, tools handle the mechanics:

1. **cocogitto (`cog bump`)** — bumps version, generates changelog, commits, tags, pushes
2. **cargo-dist** — builds cross-platform binaries, creates GitHub Release

No publishing to external registries (crates.io, Homebrew, npm) for now. `publish = false` in `Cargo.toml` blocks all registry publishing.

## Tools

### cocogitto (`cog bump`)

Automates the full release cycle: version bump → changelog generation → commit → tag → push. Determines the next version from conventional commits.

**Install:** `cargo install cocogitto`

**Config:** `cog.toml`

```toml
tag_prefix = "v"

post_bump_hooks = [
    "git push origin",
    "git push origin {{version_tag}}"
]

[changelog]
path = "CHANGELOG.md"
remote = "github.com"
owner = "edochi"
repository = "mdvs"
```

- `tag_prefix` — tags as `v0.1.0`
- `post_bump_hooks` — auto-pushes the commit and tag after bumping
- `[changelog]` — generates CHANGELOG.md with GitHub commit/PR links

See `docs/spec/cocogitto.md` for the full guide.

### cargo-dist

Builds cross-platform binaries and creates GitHub Releases. Triggered by tag pushes.

**Install:** `cargo install cargo-dist`

**Config:** `Cargo.toml`

```toml
[profile.dist]
inherits = "release"
lto = "thin"

[package.metadata.dist]
dist = true
installers = ["shell", "powershell"]
targets = [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc",
]

[workspace.metadata.dist]
cargo-dist-version = "0.31.0"
ci = "github"
```

- `[profile.dist]` — custom Cargo profile for release builds (LTO enabled)
- `[package.metadata.dist]` — package-level config (targets, installers)
- `[workspace.metadata.dist]` — workspace-level config (cargo-dist version, CI backend)

**Regenerating the workflow:** if you change the dist config, run `dist generate` to update `.github/workflows/release.yml`.

## Pipelines

### Commits (`.github/workflows/commits.yml`)

**Triggers:** push to any branch, any PR to `main`

**Steps:**
1. Checkout with full history (`fetch-depth: 0`)
2. `cocogitto-action@v4.1.0` — `cog check --from-latest-tag`

Validates that all commits since the latest tag follow conventional commit format. Runs on all branches so non-conventional commits are caught immediately, not just at merge time.

### CI (`.github/workflows/ci.yml`)

**Triggers:** push to `main`, any PR to `main`

**Steps:**
1. `cargo build`
2. `cargo test`
3. `cargo clippy -- -D warnings`
4. `cargo fmt --check`

**Runner:** `ubuntu-latest` with `rust-cache` for fast builds.

### Release (`.github/workflows/release.yml`)

**Triggers:** push of a tag matching `*[0-9]+.[0-9]+.[0-9]+*` (e.g., `v0.1.0`, `v0.1.0-rc.1`), and PRs (dry-run only)

**Jobs:**

1. **plan** (ubuntu) — runs `dist host --steps=create` to determine what to build, outputs a manifest
2. **build-local-artifacts** (4 runners in parallel) — compiles platform-specific binaries:
   - `macos-14` → `mdvs-aarch64-apple-darwin.tar.xz`
   - `macos-13` → `mdvs-x86_64-apple-darwin.tar.xz`
   - `ubuntu-22.04` → `mdvs-x86_64-unknown-linux-gnu.tar.xz`
   - `windows-2019` → `mdvs-x86_64-pc-windows-msvc.zip`
   - Each archive includes: binary, LICENSE, README.md, sha256 checksum
3. **build-global-artifacts** (ubuntu) — creates platform-agnostic installers:
   - `mdvs-installer.sh` (Unix shell installer)
   - `mdvs-installer.ps1` (Windows PowerShell installer)
   - `sha256.sum` (combined checksums)
4. **host** (ubuntu) — uploads all artifacts and creates the GitHub Release
5. **announce** (ubuntu) — final confirmation

On PRs, only the **plan** job runs (dry-run validation, no builds or uploads).

Prerelease tags (e.g., `v0.1.0-rc.1`) create a GitHub Release marked as prerelease.

## Making a Release

### Dry run (always do this first)

```bash
cog bump --dry-run --patch    # or --minor, --major, --auto
```

Shows what the next version would be without doing anything. Review with the user before proceeding.

### Release commands

| Command | Example | Use case |
|---------|---------|----------|
| `cog bump --patch` | 0.1.0 → 0.1.1 | Bug fixes |
| `cog bump --minor` | 0.1.0 → 0.2.0 | New features |
| `cog bump --major` | 0.1.0 → 1.0.0 | Breaking changes |
| `cog bump --auto` | (auto-detected) | Let commit types decide the level |
| `cog bump --pre rc` | 0.1.0 → 0.1.1-rc.1 | Prerelease / testing |

Each command:
1. Bumps version in `Cargo.toml`
2. Generates/updates `CHANGELOG.md` from conventional commits
3. Creates a bump commit
4. Tags with `v{version}`
5. Pushes commit and tag to `origin` (via `post_bump_hooks`)

The tag push triggers the Release pipeline automatically.

### `--auto` behavior

`--auto` reads commits since the last tag and picks the bump level:
- Any `feat:` → minor
- Only `fix:`, `refactor:`, etc. → patch
- Any `BREAKING CHANGE` → major

### After pushing

Monitor the release build:

```bash
gh run list --limit 1                  # find the run ID
gh run view <run-id>                   # check job status
gh run watch <run-id>                  # live follow (optional)
```

If a build fails:
```bash
gh run view --job=<job-id> --log-failed  # see failure logs
```

After success, verify the GitHub Release:
```bash
gh release view v<version>
```

### Verifying without releasing

```bash
dist plan    # shows what artifacts would be built
```

## Changelog

`CHANGELOG.md` is auto-generated by `cog bump` from conventional commits. The `[changelog]` section in `cog.toml` configures the output format, including GitHub links to commits and PRs.

The changelog groups entries by commit type (Features, Bug Fixes, etc.) and includes the full list of changes since the previous tag.

## Configuration

| File | Purpose |
|------|---------|
| `cog.toml` | cocogitto config — tag prefix, bump hooks, changelog, commit validation |
| `Cargo.toml` | Version field, `[profile.dist]`, `[package.metadata.dist]`, `[workspace.metadata.dist]` |
| `.github/workflows/release.yml` | Auto-generated by `dist generate`, do not hand-edit |

## Important notes

- `publish = false` — no crates.io publishing. Releases are GitHub-only for now.
- Tag format is `v{version}` (e.g., `v0.1.0`), configured by `tag_prefix = "v"` in `cog.toml`.
- Prerelease tags (e.g., `v0.1.0-rc.1`) create a GitHub Release marked as prerelease.
- If cargo-dist config changes, regenerate the workflow: `dist generate`

## Future: Publishing to Registries

When ready to publish externally:

1. **crates.io** — remove `publish = false` from `Cargo.toml`
2. **Homebrew** — add `"homebrew"` to `installers` and `publish-jobs` in `Cargo.toml`, create `edochi/homebrew-tap` repo
3. **npm** — add `"npm"` to `installers` and `publish-jobs` in `Cargo.toml`

Then run `dist generate` to update the release workflow.
