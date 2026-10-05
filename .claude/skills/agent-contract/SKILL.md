---
name: agent-contract
description: The standing contract every implementor and reviewer agent on mdvs must follow — tree safety, code hygiene, the real verification gate invocations, and the proof a test has to carry. Invoke this as your FIRST action when dispatched as an implementor or reviewer; the dispatching prompt carries only the task, not these rules.
---

# agent-contract

You were dispatched to implement or review one unit of change. This is the
standing contract. It is not advisory, and the prompt that sent you here does
not repeat it. Read it fully before touching anything, and confirm in your
report that you did.

`ship` describes the pipeline for the coordinator. This describes what is
required of **you**. The `rust` skill (Rust conventions) and `AGENTS.md`
(architectural invariants) also bind you.

## 1. The working tree is not yours

**Never git-mutate the tree.** No `git checkout`, `reset`, `stash`, `clean`,
`restore`, no staging, no committing, no branch changes. Read-only git —
`git status`, `log`, `show`, `diff` — is fine and encouraged.

This applies to reviewers **and** implementors. Reverting a probe file takes any
uncommitted changes in that file with it, and "cleaning up" a file you assume is
stale destroys someone else's in-progress work.

**Uncommitted files you did not create are sacred.** The tree often carries
unrelated modified or untracked files from parallel work. Do not touch them, do
not revert them, do not include them in any change you describe. If one is in
your way, say so and stop.

If you genuinely need to execute throwaway code, ask for worktree isolation
rather than writing probe files into the shared tree.

## 2. Formatting

- **Rust:** run `just fmt` after `cargo clippy` — never plain `cargo fmt`, which
  uses stable rustfmt and ignores the unstable options in `rustfmt.toml`.
  `just fmt` runs the nightly pinned in `rustfmt-toolchain`. The committed
  source is `just fmt`-canonical, so formatting should touch only what you
  changed. If it rewrites a file you did not touch, stop and report it rather
  than shipping the unrelated churn.
- **Markdown:** hard-wrapped at 80 columns by prettier. Never wrap by hand — run
  `just fmt-md <file>` on each markdown file you touched. Paths listed in
  `.prettierignore` (fixtures, `example_kb/`, the changelog) are exempt: their
  exact bytes are under test, so never format them.

## 3. Code hygiene

- **No project scaffolding in new code.** No TODO ids, no wave or phase names,
  no spec-section references, in code, comments, doc comments, test names, or
  commit-message drafts you write. Comments describe what the code does, in
  timeless prose, to a reader who has never heard of this project's plan.
  Existing references in the tree are not a precedent to copy.
- **No panics in production paths.** No `unwrap`, `expect`, `panic!`,
  `unreachable!`, `todo!`, or `assert*!` outside `#[cfg(test)]`. Propagate with
  `?` and `anyhow::Context`, use `Option` combinators, or add an explicit error
  variant. Tests may unwrap freely.
- **Respect the architectural invariants** in `AGENTS.md`: enum dispatch with
  exhaustive matches (no `dyn Trait`, no catch-all `_ =>` over our own enums);
  the validation layer (`init` / `update` / `check`) never needs the embedding
  model; strict types with coercion living in the preprocessor pipeline;
  `mdvs.toml` as the single source of truth.
- **No `#[allow(dead_code)]`.** If something is unused, either wire it up or do
  not add it. A deliberate lint suppression uses
  `#[expect(lint, reason = "...")]`, never a bare `#[allow]`.
- **No magic numbers, in production or in tests.** Every bare numeric literal
  that carries meaning (a limit, a threshold, a size, a retry count) is a named
  constant with a doc comment, or the typed value the ecosystem already
  provides. A `Duration` sits behind a named `const`, not inline.
- **Imports at the top, no wildcards.** Bring symbols in with `use` at the top
  of the module and name them bare at the use site. No `use foo::*`.
- **Let the ecosystem do the work.** Before writing a parser, a mapping, a
  mirror struct, a hand-rolled retry or a copy of a helper, check whether serde,
  the type system, the standard library or a crate already in
  `crates/mdvs/Cargo.toml` does it. A precedent elsewhere in the tree is not a
  reason to copy its shape; if the precedent is the smell, say so in your report
  rather than mirroring it.
- **Stay in scope.** Fix exactly what you were asked to fix. If you notice
  something else, put it in your report under "noticed, not fixed". Do not
  opportunistically repair adjacent code, add helper scripts, or introduce
  recipes or files nobody asked for. Downstream updates the `code-editing` skill
  describes (spec in `docs/spec/`, `book/`, `example_kb/`) are in scope only
  when your dispatch lists them; otherwise flag them under "noticed, not fixed".

## 4. STOP triggers are hard stops

Your dispatch lists STOP triggers. Hitting one means: stop, report what you
found, and wait. It does not mean route around it, and it does not mean pick the
interpretation that lets you continue.

Beyond the listed ones, always stop if the task as specified cannot work. You
are expected to push back with evidence when a specification is wrong. Show the
failing evidence rather than arguing from reasoning alone.

## 5. Verification gates

Run all of these from the repo root, in this order:

    cargo test --features testing-mocks
    cargo clippy --all-targets --features testing-mocks -- -D warnings
    just fmt
    just lint-ast
    just check-md <each .md file you touched>

- `--features testing-mocks` is not optional. Plain `cargo clippy` misses the
  mock-gated code, and the clippy invocation above is exactly what CI runs.
- `just lint-ast` runs the ast-grep rules in `.ast-grep/rules/` (panic-emitting
  calls in production code). It must report zero errors.
- **The slow lane is opt-in.** Real-model tests are `#[ignore]` and download a
  model from Hugging Face; run
  `cargo test --features testing-mocks -- --ignored` only when your dispatch
  asks for it.
- If you changed dependencies in a `Cargo.toml`, also run `cargo deny check`.
- If the change affects a command's behaviour, exercise it end-to-end against
  `example_kb/` (`cargo run -- check example_kb`, etc.) and report what you ran.
- **One cargo process at a time.** Never run two builds or test suites in
  parallel; they contend for the same `target/` lock and memory.
- **Capture exit codes properly.** This shell is zsh: `PIPESTATUS` is empty, and
  piping cargo through `tail`/`grep` reports the _pipe's_ status, so a failed
  build reads as success. Redirect to a file, check `$?`, then inspect the file.
- **Prove "pre-existing."** Never dismiss a failure as pre-existing by
  assertion. Show it lives in code you did not touch, and say when it was last
  changed.
- **Run every build and test in the foreground, with the Bash `timeout`
  parameter set (max 600000 ms); never `run_in_background` a command whose
  result you need.** You are a subagent: once you return, a background command's
  completion does not re-invoke you — the run finishes and you are never woken.
  If one call could exceed the ceiling, split it (`cargo build` then
  `cargo test`). Never return "to wait" for anything.
- If a gate genuinely cannot run, say so explicitly in your report. Never
  substitute a narrower command and describe it as the full gate.

## 6. A test must be able to fail

This is the part most often skipped, and the reason it is in the contract.

Every test you add as proof of a fix must be **shown red before the fix and
green after**. Not argued to be, demonstrated:

1. Neutralize the fix in your working copy (delete the guard, restore the old
   ordering, invert the condition).
2. Run the test. Record the **actual values** from the failure.
3. Restore the fix. Run it again.
4. Put both observations in your report, with the real numbers.

If a fixture turns out to pass under the mutation, it does not exercise the bug
— discard it and build one that does, and say in your report that you did.
Fixtures dodge bugs in ways that look fine: a single-format vault when the bug
is in mixed YAML / TOML / JSON frontmatter, ASCII-only text when the bug is
about byte-vs-character offsets, one file when the bug needs a widening event
across several.

If you cannot construct a discriminating fixture at the level you were asked to
work, say so plainly and explain what blocks it. **Do not ship a test that
passes either way.** Never fabricate a fault-injection seam to make a test
possible; if no seam exists, land the fix and report it as untestable and why.

## 7. Reporting

Return, in this order:

- Confirmation you read this contract.
- Diff shape: each file and what changed in it.
- The red-then-green proof for each exit test, with real values.
- Gate output, honestly — including anything that did not run and why.
- Judgement calls you made, especially any deviation from your dispatch and the
  evidence for it.
- "Noticed, not fixed."

Do not commit. Do not stage. The human approves every commit.
