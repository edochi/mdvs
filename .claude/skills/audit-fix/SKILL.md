---
name: audit-fix
description: Diagnose and fix a failed Audit workflow run (cargo audit / cargo deny check) — RustSec advisories, yanked crates, unmaintained notices, license or ban failures. Use when the user mentions a failed Audit run, a RUSTSEC ID, or cargo audit / cargo deny failing in CI or locally. Covers branch and PR naming, the fix decision order, verification, and the PR description template.
---

# Audit Fix

`.github/workflows/audit.yml` runs `cargo audit` then `cargo deny check` every
Monday at 06:00 UTC. `ci.yml` runs the same two commands on every PR, so a new
advisory turns **every open PR red at once**, whatever code it touches. Fix it
fast, in a minimal PR that merges first.

Precedents: PR #68, #72, #76.

## Conventions

| What                   | Pattern                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| Branch                 | `fix/audit-YYYY-MM-DD`                                               |
| PR title / commit (1)  | `fix(deps): bump <crate> to <version> for RUSTSEC-YYYY-NNNN`         |
| PR title / commit (2+) | `fix(deps): bump <crate> to <version> for <N> RustSec advisories`    |
| Several crates         | `fix(deps): bump <crate-a> and <crate-b> for <N> RustSec advisories` |

- **Date** is the UTC date of the failed run (`gh run list` shows it). If the
  failure was found locally or in PR CI with no scheduled run, use the date it
  was first seen. If that branch already exists, append `-2`.
- **Ignore-only fixes** (no patched version exists):
  `fix(deps): ignore RUSTSEC-YYYY-NNNN (<crate>, no fix available)`.
- Always ask the user before creating the branch (AGENTS.md).

## Workflow

### 1. Diagnose the run

```bash
gh run list --workflow Audit --limit 5
gh run view <run-id>                       # which step failed: audit or deny?
gh run view <run-id> --log-failed | sed 's/\x1b\[[0-9;]*m//g' \
  | grep -E 'Crate:|Version:|ID:|Title:|Severity:|Solution:|error|warning'
```

If `cargo audit` fails, `cargo deny check` never runs, so CI shows only half the
picture. Always reproduce both locally.

### 2. Reproduce on an up-to-date main

```bash
git checkout main && git pull --ff-only
cargo audit
cargo deny check
```

Advisories published after the run may show up locally. Fold them into the same
PR (as #68 did) so the workflow goes fully green.

### 3. Trace each flagged crate

```bash
cargo tree -i <crate>@<version> --edges normal,build | head -30
```

Record the shortest path from `mdvs` to the crate. It goes in the PR table and
tells you who constrains the version.

### 4. Pick the fix, in this order

1. **Targeted lockfile bump** (the default):

   ```bash
   cargo update -p <crate>@<old-version> --dry-run   # check what moves
   cargo update -p <crate>@<old-version>
   ```

   Lists everything that moved. Anything besides the flagged crate goes under
   "Incidental changes". Never run a bare `cargo update` here: general bumps go
   in a separate `chore(deps)` PR.

2. **Manifest change.** If the fixed version is semver-incompatible or a parent
   pins it, find which parent constrains it and whether a newer parent release
   lifts the pin. A direct-dependency bump in `crates/*/Cargo.toml` is a design
   decision: **stop and discuss with the user** before editing.

3. **Ignore.** Only when no fixed version is reachable, and **only with the
   user's explicit approval**. Add it to `deny.toml` `[advisories].ignore`,
   grouped under a comment naming the direct dependency it comes through. The
   reason must state three things:

   ```toml
   { id = "RUSTSEC-…", reason = "<crate> is unmaintained; pulled in via <direct dep> → … → <parent> <ver>. Revisit when bumping <direct dep>: <parent> <ver> no longer depends on it." },
   ```

   If no parent release has dropped it yet, say so:
   `Revisit when <parent> drops it (latest <ver> still depends on it).`

   Keep the reason self-contained: name crates and versions, never a TODO, issue
   or PR. Those close or move, and the config outlives them. Upgrade work is
   tracked in a TODO that points at `deny.toml`, not the other way round.

   `cargo audit` does not read `deny.toml`. Unmaintained notices are only
   warnings there, so `deny.toml` is enough. An ignored **vulnerability** also
   needs `.cargo/audit.toml` `[advisories] ignore = ["RUSTSEC-…"]` (create it if
   missing).

**Unmaintained notices** are warnings in `cargo audit` but errors in
`cargo deny`, so a new one can stay hidden in CI until `cargo audit` passes.
Always run both locally. A new one needs the user's call (replace the dep, or
ignore it with a reason).

**License / bans / sources failures** in `cargo deny` are not advisories: show
the user the failure and the dependency path, then discuss.

### 5. Re-check existing ignores

`deny.toml` sets `unused-ignored-advisory = "deny"`, so an entry that no longer
matches anything fails CI and has to be removed. That only catches fixes that
have already reached our lockfile. Fixes that need a direct-dependency bump are
caught here, on every audit-fix run that touches `deny.toml`.

For each ignore entry, start from the **latest release of our direct
dependency** (`lancedb`, `model2vec-rs`, `tabled`, …) and walk down the path
from `cargo tree -i <crate>`. At each hop, take the version the previous crate
requires (not the hop's own latest release) and list its dependencies:

```bash
UA="mdvs-audit-check (github.com/edochi/mdvs)"   # crates.io requires a UA
deps() {  # deps <crate> <version> <regex>
  curl -s -A "$UA" "https://crates.io/api/v1/crates/$1/$2/dependencies" \
    | jq -r --arg f "$3" '.dependencies[] | select(.crate_id|test($f))
        | "\(.crate_id) \(.req) (\(.kind)\(if .optional then ", optional" else "" end))"'
}
curl -s -A "$UA" https://crates.io/api/v1/crates/lancedb | jq -r .crate.max_stable_version
deps lancedb 0.40.0 '^lance$'                 # → lance =13.0.0
deps lance-tokenizer 13.0.0 '^lindera$'       # → lindera ^3.0.7 …
```

The ignore can be cleared once some hop in the chain no longer requires the next
one. Checking only the immediate parent's latest release is wrong: our direct
dependency may still require an older version of that parent (e.g.
`model2vec-rs` 0.3.0 still requires `tokenizers` 0.21, even though newer
`tokenizers` releases exist). Note `optional` dependencies: turning off a
feature may be enough to drop the crate. Update the entry's "Revisit when" text
and the "last checked" date in the comment above the list, and report it in the
PR's "Ignored advisories" section. **Don't bump the direct dependency in the
audit-fix PR.** Propose a separate `chore(deps)` PR or TODO to the user instead.

### 6. Verify

Run all of these and record the results for the PR:

```bash
cargo audit
cargo deny check
cargo test --features testing-mocks
cargo clippy --all-targets --features testing-mocks -- -D warnings
just fmt-check
just lint-ast
```

The real-model lane (`-- --ignored`) is optional for lockfile-only changes. Run
it when a manifest changes or the bump touches the embedding stack.

### 7. Blocked PRs

```bash
gh pr list --state open
gh pr checks <number>          # for each open PR
```

If other open PRs fail CI's `audit` job on the same advisory, add the "Merge
order" section to the PR description.

### 8. Commit and PR

Commit only when the user says so (`commit` skill). Stage `Cargo.lock` (and
`deny.toml` / `.cargo/audit.toml` / manifests if touched) by name.

## PR description template

Fill every section; drop "Merge order" when nothing else is blocked. Link each
advisory to `https://rustsec.org/advisories/<ID>`.

```markdown
## Summary

The scheduled Audit workflow failed on `main`
([run <id>](https://github.com/edochi/mdvs/actions/runs/<id>), <date>) at
`cargo audit` | `cargo deny check`. <One line: lockfile-only bump / manifest
change / ignore, and why.>

## Advisories

| ID         | Crate | From → To | Severity    | Title | Pulled in via                   |
| ---------- | ----- | --------- | ----------- | ----- | ------------------------------- |
| [RUSTSEC-] |       |           | 4.0 medium  |       | `lancedb → … → <crate>`         |

## Incidental changes

<Other lockfile movement from re-resolution and why it's harmless, or "None —
only `<crate>` moved.">

## Verification

| Check                                                        | Result |
| ------------------------------------------------------------ | ------ |
| `cargo audit`                                                |        |
| `cargo deny check`                                           |        |
| `cargo test --features testing-mocks`                        |        |
| `cargo clippy --all-targets --features testing-mocks -- -D warnings` | |
| `just fmt-check`                                             |        |
| `just lint-ast`                                              |        |

## Ignored advisories

<Entries added or removed in `deny.toml` and why. Then the re-check result for
every remaining entry: "parent X <ver> still depends on it" or "dropped in
<parent> <ver> — needs a <direct dep> bump (proposed as separate work)".>

## Merge order

<Only when other open PRs are red on the same advisory: list them; this PR goes
in first.>
```
