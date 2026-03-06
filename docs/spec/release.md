# Release Process

## Overview

Releases are semi-automated. You decide when to release, tools handle the mechanics:

1. **cargo-release** — bumps version, commits, tags, pushes
2. **cargo-dist** — builds cross-platform binaries, creates GitHub Release
3. **cocogitto** — enforces conventional commits, generates changelog

No publishing to external registries (crates.io, Homebrew, npm) for now. `publish = false` in `Cargo.toml` blocks all registry publishing.

## Tools

### cargo-release

Automates the version bump → commit → tag → push cycle.

**Install:** `cargo install cargo-release`

**Config:** `release.toml`

```toml
publish = false       # no crates.io publishing
push-remote = "origin"
```

The tag format is `v{version}` (e.g., `v0.1.0`). This is the default — no `tag-prefix` needed.

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

### cocogitto

Enforces conventional commits and generates changelog.

**Install:** `cargo install cocogitto`

**Config:** `cog.toml`

See `docs/spec/cocogitto.md` for the full guide.

## Pipelines

### Commits (`.github/workflows/commits.yml`)

**Triggers:** push to any branch, any PR to `main`

**Steps:**
1. Checkout with full history (`fetch-depth: 0`)
2. `cocogitto-action@v3` with `check-latest-tag-only: true`

Validates that all commits since the latest tag follow conventional commit format. Requires full git history to locate the tag. Runs on all branches so non-conventional commits are caught immediately, not just at merge time.

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

### Patch release (bug fixes)

```bash
cargo release patch --execute
```

Bumps `0.1.0` → `0.1.1`, commits, tags `v0.1.1`, pushes. The release pipeline builds and publishes automatically.

### Minor release (new features)

```bash
cargo release minor --execute
```

Bumps `0.1.0` → `0.2.0`.

### Major release (breaking changes)

```bash
cargo release major --execute
```

Bumps `0.1.0` → `1.0.0`.

### Release candidate

```bash
cargo release rc --execute
```

Bumps `0.1.0` → `0.1.1-rc.1`. Creates a prerelease GitHub Release.

### Dry run

All commands default to dry-run. Omit `--execute` to see what would happen without doing anything:

```bash
cargo release patch    # shows what it would do
```

### Verifying without releasing

```bash
dist plan    # shows what artifacts would be built
```

## Changelog

Cocogitto generates `CHANGELOG.md` from conventional commits when bumping versions:

```bash
cog bump --auto    # determine version from commit types
cog bump --patch   # force patch bump
```

This is separate from `cargo release` — use one or the other for version bumping, not both. Currently we use `cargo release` for bumping and will integrate changelog generation later via `pre_bump_hooks` in `cog.toml`.

## Future: Publishing to Registries

When ready to publish externally:

1. **crates.io** — remove `publish = false` from `Cargo.toml` and `release.toml`
2. **Homebrew** — add `"homebrew"` to `installers` and `publish-jobs` in `Cargo.toml`, create `edochi/homebrew-tap` repo
3. **npm** — add `"npm"` to `installers` and `publish-jobs` in `Cargo.toml`

Then run `dist generate` to update the release workflow.
