---
name: spec
description: Spec document conventions for writing and reviewing specification documents. Apply when creating or modifying specifications, workflows, API docs, or terminology definitions.
---

# Spec Document Conventions

If the project has an established spec directory structure (e.g., in CLAUDE.md or a README), follow that organization. The checklist below is general-purpose.

## Spec Definition Checklist

### Phase 1: Classify the Document

- [ ] Determine which category it belongs to:
  - **Terminology** — Canonical definitions (single source of truth for terms)
  - **Crate/module specs** — Per-component state, messages, constants
  - **Workflows** — Cross-component flows and sequences
  - **Security** — Authentication, encryption, authorization
  - **Operations** — Telemetry, testing, persistence, versioning
  - **Infrastructure** — Databases, deployment, networking

### Phase 2: Write the Document

- [ ] Add status header: `**Status: DRAFT**` (or REVIEW, FINAL)
- [ ] Add cross-reference links at the top to related specs
- [ ] Use Mermaid for all diagrams — never ASCII art
- [ ] Define new types in the appropriate component spec, not inline in workflows
- [ ] Use consistent table format for message definitions, state transitions, constants

### Phase 3: Cross-Check for Consistency

- [ ] **Terminology:** Are all terms used per their canonical definitions?
- [ ] **Component specs:** Do referenced fields/messages actually exist in their spec?
- [ ] **Workflows:** Does this flow contradict any existing workflow?
- [ ] **Constants:** Are constant names and values consistent across documents?
- [ ] **State transitions:** Are state machine transitions consistent with their spec?

### Phase 4: Finalize

- [ ] Extract any new data structures -> add to the appropriate component spec
- [ ] Update terminology doc if new terms introduced
- [ ] Add Related Documents section at the bottom with links to all referenced specs
- [ ] If this is a workflow: add Actors table, End States table, Edge Cases section

## Structural Rules

- **Single source of truth:** each term/struct/message defined in ONE place only
- **Cross-references:** link, never duplicate definitions
- **Component specs should mirror the code structure** (crate layout, module hierarchy)
- **Workflow documents should include:** Overview, Actors, Sequence Diagram, POV sections (per actor), Edge Cases, Messages, Constants, Related Documents
