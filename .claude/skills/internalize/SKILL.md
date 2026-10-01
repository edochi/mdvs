---
name: internalize
description: The "internalize" step of a design→plan→implement→validate→internalize→save pipeline. Given a unified diff and read access to the code it touches, produce a self-contained interactive HTML explainer built for OWNERSHIP — understanding and being able to steer the change — NOT correctness checking (that's the validate & review stage). Use when asked to explain/internalize a diff, understand what an agent just changed, or build ownership of agent-written code before saving. Adapted from Geoffrey Litt's /explain-diff, extended with a decision log and a judgment quiz.
---

# internalize

## Purpose

Agents write code faster than humans can absorb it, so _understanding_ — not
writing — is the bottleneck. The durable reason to understand isn't to
**verify** (that's the `validate` step; agents increasingly self-verify) but to
**participate**: you can't steer the next iteration of a system you don't hold
in your head, and you can't feel ownership of decisions you never saw made.

This skill is a **stateless transform**: given a diff and the code it touches,
it emits an interactive HTML explainer that rebuilds understanding fast. It does
_not_ decide how the diff was obtained, where the output lives, or when to run —
those are the caller's concerns (see Inputs / Output).

## Inputs (contract)

The caller supplies:

- **A unified diff** — from any source: `git diff` (working tree / staged / a
  commit range `A..B` / a PR), a raw patch piped in, or the output of a delta
  tool. The skill treats it as opaque unified-diff text; it does not care how it
  was produced.
- **Read access to the touched code** — enough of the repo/tree to explore
  callers, callees, types, and config so the Background is real, not a
  paraphrase of the diff.
- _(optional)_ **Technical-depth hint** — `line` / `module` / `architecture`.
  Tunes how granularly the code is walked, _not_ whether the wider objective is
  explained (that's always covered). If absent, infer from diff size (see
  Guidelines).
- _(optional)_ **Output target** — where to write the artifact. If absent, the
  skill writes to a scratch path and returns it; deciding the durable home is
  the caller's job.

If invoked ad-hoc with none of these, derive a convenience default (e.g.
`git diff` of the current branch vs its merge-base) — but treat that as a
fallback, not part of the skill's responsibility.

## Guidelines

**Grounding — this skill builds _ownership_, so it must never manufacture it.**
Every factual claim about the existing system (Background) and every asserted
rationale (Decision log) must trace to code you actually read, or to the diff,
commits, or comments themselves. Tag each non-obvious claim by its epistemic
tier: **(stated)** — present in the diff / comment / commit message;
**(inferred)** — your read of the evidence, cite the file or symbol that
supports it; **(open question)** — genuinely unknowable from what you can see,
flag it rather than guess. Never present an inference or a guess as a stated
fact. An explainer that fabricates the "why" is worse than none — the reader
ends up owning a fiction and steers the next iteration wrong.

**Two independent axes — never conflate them.**

- **The wider objective (the "why") is always covered, and usually leads —
  independent of diff size.** Why does this change exist, what larger goal or
  roadmap step it advances, what it unblocks or sets up next. This is frequently
  the _most important_ part of the whole explainer: a one-line flag flip can
  carry more strategic weight than a 500-line refactor. Never let a small diff
  collapse into "just the technical delta." If anything, a _small_ diff frees up
  room to go **wider** on objective and context, not narrower.
- **Technical walkthrough depth scales inversely with diff size.** This axis
  governs _only_ how granularly the Literate diff walks the code and how fine
  the code-track quiz questions get. Detect size from _human-authored_ LOC
  changed + files touched + hunk count — **exclude generated, vendored, and
  lockfile hunks from the count** (a 3,000-line lockfile churn is a _small_
  semantic change and must not be pushed into the architecture bucket).
  Thresholds tunable:

  | Size (rough)             | Technical depth of the literate diff / code-track questions                      |
  | ------------------------ | -------------------------------------------------------------------------------- |
  | ≲150 LOC, few files      | Walk essentially every hunk; concrete, line-level.                               |
  | ~150–800 LOC             | Function/module level; group by unit of behaviour.                               |
  | ≳800 LOC or multi-commit | _Sample_ the most consequential changes; name what you skipped; stay structural. |

The Background, Intuition, and Decision-log sections — the objective and the
reasoning — stay first-class at _every_ size. A caller-provided depth hint
overrides the inferred size bucket; it tunes technical depth, not whether the
objective is explained. If the diff is larger than you can hold in context,
prioritize by the objective: walk what serves the "why" and explicitly list the
rest as un-walked.

**Style:** clear, flowing, example-first prose in the vein of Martin Kleppmann.
Concrete over abstract; every concept gets toy data; smooth transitions.

**Diagrams:** HTML/CSS, never ASCII. Reuse a small number of diagram families,
picking the one that fits the change: a simplified UI mock for UI changes; a
data-flow/component diagram for wiring changes; a before/after trace of example
data through the code for algorithmic, type, or config changes. Always include
example data.

**Code blocks:** always `<pre>` (or a div with `white-space: pre-wrap` in its
CSS — otherwise the browser collapses newlines). Before emitting, scan every
code block and confirm it has `white-space: pre` or `pre-wrap`.

Use callouts for key concepts, definitions, and important edge cases.

**Degenerate & mixed diffs — resolve these before applying the template:**

- **Empty diff** → say so and return; produce nothing.
- **Formatting / whitespace-only** → collapse the technical walkthrough to one
  line ("no behavioural change") and spend the budget on _why_ the reformat
  happened. Suppress the quiz.
- **Generated files / lockfiles / vendored code** → summarize as "N generated
  files changed as a consequence of X; not walked." Never line-walk them,
  whatever the size.
- **Pure deletions** → the objective _is_ the story: why remove this, what
  depended on it, what replaces it. Background covers what's being lost.
- **Renames / moves** → treat as structural; don't render as delete-plus-add.
- **Touched code you cannot read** → mark it in Background as "couldn't read X;
  treated as a black box" — never invent it.

## Template

The generated page has these sections, in order. Do **not** prepend a standalone
orientation or "read this first" preamble — start directly with Background:

1. **Background** — The existing system the change touches. A _collapsible_
   deep-background block for someone new to the area (skippable), then a narrow
   background directly relevant to the change. Explore the surrounding code to
   write this.
2. **Intuition** — Open with the **wider objective**: why this change exists and
   what it moves the system toward — always, and _especially_ when the diff is
   small (a tiny change often carries the biggest intent). Then the _essence_ of
   the mechanism, not the details: concrete toy-data examples and diagrams. This
   is where understanding forms.
3. **Literate diff** — Walk the changes in a _sensible conceptual order_ (never
   files alphabetically), grouped by idea, with prose around embedded, annotated
   code snippets. At `architecture` altitude, walk representative changes and
   name what was sampled out.
4. **Decision log** — THE ownership section, and the hardest to do well — so
   scaffold it. First _find_ the real decisions: forks where the code could
   plausibly have gone another way and the choice has consequences — a
   non-obvious data structure or algorithm, an error-handling strategy
   (propagate vs. swallow vs. retry), the shape of a boundary or interface, a
   backward-compat or migration accommodation, a name that encodes a concept,
   something added defensively, or something conspicuously _not_ done. Prefer
   **2–4 genuine decisions over an exhaustive list of trivial ones** (skip
   "chose a `Vec`"), most consequential first; if the diff holds no real
   decision, say so rather than manufacture one. For each: **the decision**,
   **options considered**, **what was chosen**, **why** (grounded per the
   Grounding rule — tag stated / inferred / open), and **what we gave up** (the
   tradeoff). This turns a reviewer into a participant.
5. **Quiz** — questions across two tracks, with **at least one judgment
   question** (it's the differentiator). **Scale the count to the
   technical-depth axis:** ~5 at `line`, ~8 at `module`, ~10+ at `architecture`.
   Skip the quiz entirely for no-behavioural-change diffs. Start directly with
   the first question — no intro, "two tracks" preamble, or instructions line.
   - **Code track** — what the code now does (medium difficulty: needs real
     understanding, no gotchas).
   - **Judgment track** — the decisions ("why approach A over B", "what breaks
     under the alternative"). This is what makes you feel you'd have made the
     same call.

   **Resist gameability (hard requirement, not a nicety):**
   - **No length/specificity tell.** Every option in a question must be
     comparable in length and concreteness; the correct answer must _never_ be
     the longest or most-detailed. Distractors are _specific, plausible
     misconceptions_ a partial-understander would actually pick — never vague
     filler.
   - **Mix formats**, don't ship uniform single-answer multiple-choice: include
     some **"which statement is FALSE"** items, a couple of **true/false** on a
     precise claim (inherently length-neutral), and **one "select all that
     apply"** (multi-select) for the judgment track (harder to game; self-scores
     all-or-nothing). Keep some standard single-answer MC with disciplined
     distractors.
   - **Shuffle option order** client-side at render so position is never a tell
     either.
   - **No open-ended/free-text** questions: a self-scored client-side page can't
     grade prose.
   - **Multi-select machinery — the one that keeps getting forgotten.** Every
     "select all that apply" question needs its own explicit
     `<button class="check">Check</button>` immediately before its feedback
     element — the scoring engine binds to it, so without it the question is
     unscorable. Single-answer questions must NOT have one. Two guards: (1) make
     the engine tolerate a missing `.check` (guard the `querySelector('.check')`
     so a missing one degrades gracefully instead of throwing and killing later
     questions); (2) **after every render, verify
     `count(<button class="check">) == count(multi-select questions)`** before
     considering the artifact done — this is a required post-render check, not
     optional.

   Interactive; clicking reveals correct/incorrect **with per-option feedback**;
   self-scores (multi-select scored all-or-nothing). **Non-blocking** — never
   hard-stops; end with the score, no closing exhortation.

## Output (contract)

- Emit **one self-contained HTML file**: all CSS and JS inline, no external
  requests. One long page with a header and a table of contents; no top-level
  tabs. Basic responsive styling.
- Write it to the caller's output target if given, else a scratch path.
- If the caller specifies `<head>` metadata (title, `<meta>` tags), emit it
  exactly as given.
- **Return the artifact path** and stop. Whether to open it, where to archive
  it, and whether to track it are the caller's decisions.

## Notes

- Adapted from Geoffrey Litt's `/explain-diff` (HTML variant). The deltas —
  decision log, judgment quiz track, non-blocking gate, and the stateless
  input/output contracts — make it composable and ownership-focused.
- The explainer is a lens, not a replacement for the code — the reader still
  reads the raw diff afterward. This is a working principle, not something to
  state on the page as a preamble.
