---
name: ship
description: Orchestrates one unit of change (a "wave") through a disciplined design→plan→implement→validate→save cycle, keeping the human in the decision seat. One implementor agent writes at a time; adversarial reviewers attack the result; the coordinator verifies everything. An optional internalize stage (only when the user explicitly asks) turns the human from approver into participant before save. Use when driving a non-trivial change end-to-end with an agent doing the writing.
---

# ship

## Why this exists

Agents write code faster than a human can absorb it, so the bottleneck is no
longer _writing_ or even _verifying_ (agents increasingly self-verify) but
**ownership**: you cannot steer the next iteration of a system you do not hold
in your head, and you cannot feel responsible for decisions you never saw made.

Left unchecked, the loop collapses to "agent writes → coordinator validates →
human rubber-stamps a commit they never read." `ship` prevents that. It routes
every change through explicit gates the human owns.

`ship` is the orchestrator. It does not itself explain diffs, run reviews, or
implement code. It sequences those, and — when the human asks for it — invokes
the `internalize` skill at the ownership gate.

## The pipeline

Six stages. The **gate owner** is who must act before the stage advances.

| Stage                    | What happens                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Gate owner                               |
| ------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------- |
| 1. **Design**            | Surface every real decision as a decision, with a recommendation. **Always run the `ship-designer` agent before presenting the forks to the human** — it grounds the options and hunts for the flaw that sinks the design. Then the coordinator **double-checks its every load-bearing claim against the real code** (agent output is a hypothesis) before relaying or acting. The agents NEVER decide a fork.                                                                                                                                                                                                                                                                                                                                                                                                                                                 | **Human** decides each fork.             |
| 2. **Plan**              | Draft the implementation agent's prompt with the decisions baked in, STOP triggers enumerated, the anti-patterns forbidden, and the downstream updates the change needs (spec, book, `example_kb` — see the `code-editing` skill). Show the scope to the human for anything non-trivial.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Coordinator drafts; human sees scope.    |
| 3. **Implement**         | **One** `ship-implementor` agent writes the change and, _before returning_, runs the full local gate itself (tests + clippy + fmt + ast-grep) and reports the results. Never parallel implementors. (Parallel read-only investigation is fine and encouraged.)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Coordinator runs it.                     |
| 4. **Validate & review** | Coordinator verifies the output against the real code (agent output is a _hypothesis_ — check every non-trivial file:line / API claim) and re-runs the gate, classifying any failure as pre-existing (prove it) vs real. Then **`ship-reviewer` agent(s)** attack the change, one lens each; scale their number and lenses to scope: 1 for a small change; 2–3 distinct lenses (correctness, layering/API, test-quality/bug-immunity) for complex or coupled work, and the **simplicity/idiom** lens on every new module or any diff over a few hundred lines: is this the least code that does the job, what would a senior Rust reviewer delete, is serde / the type system / std doing the work instead of hand-written code. Implementor + reviewer(s) form a persistent **wave team** that loops fix→re-review until the reviewers are clean (see below). | Coordinator routes + verifies.           |
| 5. **Internalize**       | **Opt-in — skipped unless the human explicitly asks for it.** When requested: commit locally first (human-approved) so the explainer anchors to a stable SHA, then dispatch `ship-internalizer` on that commit's diff. The human reads the explainer and takes the judgment quiz. This is where approver becomes participant.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | **Human** internalizes (when requested). |
| 6. **Save**              | Commit, push, open or update the PR, or move to the next wave — each on the human's explicit word.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | **Human** authorizes commit / push / PR. |

The two things that make this different from "just use an agent": the **Design
gate** (the human decides the forks, not the agent) and the **Internalize gate**
(the human understands the diff before it leaves the machine — run it whenever
the human asks for it).

## The wave team (implement + review loop)

Stages 3–4 run as a small, persistent **team** for the wave, not one-shot calls.
Agents keep their context after they return and are resumed by id (via
`SendMessage`), so the team iterates without re-explaining itself:

1. Spawn the **implementor** (`ship-implementor`; writes + self-runs the gate).
   When it returns, spawn the **reviewer(s)** (`ship-reviewer`, one lens each,
   scaled to scope). Hold every agent's id. The coordinator still verifies every
   high-impact finding itself before it becomes a fix.
2. Reviewers return findings. The coordinator **triages them before any reach
   the implementor**: verify each against the real code (scrutinize _critical /
   behavior-changing_ findings hardest, since adversarial reviewers have a
   higher false-positive rate), and **filter out the over-harsh, the stylistic
   nitpick, and the not-actually-necessary**. Only real, worth-fixing findings
   are relayed, rephrased as concrete asks. A reviewer false-positive or an
   unnecessary demand dies here, not in the implementor's queue.
3. Resume the **same implementor** with the verified fixes; it still holds the
   full context of what it built.
4. Resume the **same reviewer(s)** to check whether each finding was actually
   addressed (they remember what they flagged, so re-review is cheap and
   honest).
5. Loop 3–4 until the reviewers are clean and the coordinator is satisfied. Then
   the wave moves on to save (or to the internalize gate, if the human asked for
   it). A **new wave starts a fresh team**; the previous team is abandoned.

Bounds and discipline:

- **Protect uncommitted work from reviewers.** Reviewers must NOT modify the
  shared working tree — no throwaway edits, even ones they promise to revert (a
  revert of a reviewer's probe file clobbers the implementor's _uncommitted_
  changes in that same file). Reviewers are read-only; if one genuinely needs to
  execute probe code, spawn it with **worktree isolation**. Committing the
  implementor's work before review is the stronger guard, but it needs the
  human's explicit "commit" (see Commit discipline) — absent that, rely on
  read-only reviewers plus worktree isolation, and never stash on the human's
  behalf.
- **Hard cap: two fix→re-review rounds.** If the reviewer(s) still block after
  the second iteration, STOP the loop and bring the open issues to the human to
  decide together. The loop must never become a blocker — escalation is the
  release valve, not endless iteration.
- **Filter, don't relay raw.** Adversarial reviewers are deliberately harsh and
  will flag things that don't need fixing. The coordinator decides what is worth
  acting on; the implementor sees a curated set of verified issues, never the
  raw adversarial dump.
- The coordinator is the router **and** the verifier, never a blind pipe: fixes
  are verified before re-review, findings are verified _and filtered_ before
  becoming fixes.
- If reviews keep exploding within the two rounds, the scope was too big — split
  the wave (agent context also grows each round).

## The agent roster

The agents live in `.claude/agents/` and pin their own model and effort:

- `ship-designer` — design review, read-only. `ship-designer-opus` is the same
  contract on the Opus tier, for a second, independent design opinion.
- `ship-implementor` — the single writer, for waves that leave judgment to the
  implementor. `ship-implementor-sonnet` is the same contract on a cheaper tier
  at high effort, for waves whose design is already a checklist (sites, tests
  and fixtures enumerated, one crate).
- `ship-reviewer` — adversarial, read-only, one lens per dispatch.
- `ship-internalizer` — the explainer (stage 5, only when requested).

Dispatch them by name; override effort per dispatch only when a specific wave
needs it, and say why in the prompt.

## The agent-facing contract

The implementor and reviewer agents preload the **`agent-contract`** skill (and
the `rust` skill). That skill is the single source of the rules the _agent_ must
follow — tree safety, code hygiene, the real gate invocations, and the
red-then-green proof a test has to carry. Do not restate those rules in dispatch
prompts: a hand-copied contract drifts, and a dropped line costs real work. The
prompt carries the **task**; the skill carries the **contract**.

The rules below are the coordinator's own, and are not duplicated there.

## Operating rules (load-bearing)

These are not optional; they are why the pipeline produces trustworthy output.

- **Surface decisions, do not make them.** When the agent (or you) hits a fork
  the human hasn't ruled on, STOP and present it with a single recommendation.
  Never resolve a design fork unilaterally mid-implementation.
- **Design is reviewed first, then verified.** Before presenting the design
  forks to the human, dispatch `ship-designer` on the proposed shape, for every
  design regardless of apparent difficulty. Its job is to ground the options and
  find the flaw that sinks the design: an architectural invariant broken (enum
  dispatch, the validation/search layer split, strict types — see `AGENTS.md`),
  a function or type that doesn't exist, a pipeline stage that won't see the
  change. Then double-check its every load-bearing claim against the real code
  before you relay or act; the agent's output is a hypothesis, and a wrong claim
  relayed as fact steers the human into a bad decision.
- **One implementor at a time; reviewers may be several.** Serialize
  _implementation_ — a single implementor writes the change. Reviewers are
  separate agents and may run in parallel with distinct adversarial lenses. Keep
  the implementor and reviewers in **separate contexts** so the review stays
  adversarial; the coordinator relays _verified findings_ between them, never
  the implementor's rationalizations.
- **Agent output is a hypothesis.** Before acting on any agent claim (a
  file:line, an API signature, "this is a no-op"), read the real file. Agents
  fabricate plausible specifics.
- **Prove "pre-existing."** A failing test is not dismissed as
  flake/pre-existing by assertion. Reproduce it on the untouched baseline before
  classifying it out.
- **Branch before the first edit.** All work goes on a feature branch (ask the
  human before creating one). Never edit on `main`.
- **Commit discipline.** Follow the `commit` skill. Never commit or push without
  the human's explicit word — a commit needs "commit" in their latest message, a
  push needs "push". Present the message + the exact staged file list first.
  `git add` / `git commit` / `git push` are separate calls, never chained. Check
  `git status` and unstage anything unrelated before committing. Conventional
  commit types follow the change, not the branch name — cocogitto derives the
  version bump from them.
- **No project scaffolding in new code.** No TODO ids, wave or phase names, or
  spec-section references in code or comments you add. Comments are timeless
  descriptions of what the code does. Enforce this in every implementation
  prompt and grep the diff for it before the commit.
- **No panics in production.** No `unwrap` / `expect` / `panic!` /
  `unreachable!` in production paths (`just lint-ast` enforces this). Bake the
  ban into the implementation prompt.

## The internalize gate (stage 5, opt-in)

**Skip this stage unless the human explicitly asks for it** ("internalize this
wave", "run internalize"). When they do, sequence:

1. **Propose the commit.** Present the commit message + the staged file list to
   the human. Wait for explicit approval.
2. **Commit locally.** A local commit publishes nothing and is easily fixed up,
   so if internalize surfaces a problem you fix it before push.
3. **Dispatch `ship-internalizer`** on that commit's diff
   (`git diff <base>..<commit>`), with read access to the touched code and the
   output path `docs/internalize/<slug>.html`. It follows the `internalize`
   skill and writes one self-contained HTML file (see Artifacts). It does not
   decide where things live or when to run — those are this skill's job.
4. **The human internalizes.** They open the file
   (`open docs/internalize/<slug>.html`), read the explainer, take the judgment
   quiz and write reactions. Passing the quiz is not required, but if the human
   cannot, they do not yet own the change — a signal to slow down, not to push.
5. **Then, and only then, stage 6:** push / move to the next wave, on the
   human's word.

If internalize (or the human's reactions) surfaces a real problem, loop back:
fix the change, regenerate the explainer, re-read. Nothing has been pushed.

## Artifacts

- Explainers are **tracked** in `docs/internalize/`, one self-contained `.html`
  per wave (`<slug>.html`), committed alongside the wave they explain. This
  repository is public, so they are published with it.
- **Metadata lives in the HTML `<head>`** as `<meta>` tags, so the record stays
  greppable:

  ```html
  <title><wave> — <short description></title>
  <meta name="description" content="One-line what-and-why for search." />
  <meta name="ship:commit" content="<sha>" />
  <meta name="ship:base" content="<sha>" />
  <meta name="ship:created" content="<YYYY-MM-DD>" />
  <meta name="keywords" content="<searchable>, <keywords>" />
  ```

  `commit` is the commit the explainer anchors to; `base..commit` is the diff
  range it explains.

## Closing a TODO (the range gate)

A TODO is not marked `done` on the strength of its waves' reviews. Each wave's
reviewers saw a diff and a lens; nobody saw the composed system. Before `done`:

1. **Whole-range review.** Dispatch a read-only `ship-reviewer` over the TODO's
   full commit range with the lenses that matter here: correctness of the
   composed system across every command and pipeline stage it touches
   (`discover → schema/config → validation → storage → search → output`);
   completeness against the TODO's scope; alignment with the surrounding code; a
   predicate audit of every site answering the same question; test
   discrimination for every guard; record vs code (spec, book, `example_kb`);
   house rules. Verify its load-bearing claims against the code before acting.
   Its findings are waves or deferrals, never a footnote.
2. **Resolution checked claim by claim.** Every sentence in the TODO's
   Resolution that says what the code does is verified against the code before
   it is written, not after.
3. **Every deferral has a home** that actually contains it: grep the target TODO
   for the item.

Then close it with the `todo` skill.

## Scope notes

- `ship` is deliberately thin. The intelligence is in the stages: the design
  discussion, the implementation prompt, the validation, and (when requested)
  `internalize`.
- Use it for a _unit of change_ (one wave / one coherent sub-wave), not a whole
  epic. Chain it: one `ship` cycle per wave, human in the loop between them.
- For trivial mechanical edits it is overkill. Reach for it when an agent is
  writing something you will later need to steer.
