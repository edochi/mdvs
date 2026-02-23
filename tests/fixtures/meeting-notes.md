---
title: "Search Ranking Sync -- 2025-06-03"
date: 2025-06-03T10:00:00
type: meeting
attendees:
  - edoardo
  - sara
  - marco
project: mdvs
tags: [meeting, search, ranking]
---

# Search Ranking Sync

**Date:** 2025-06-03 10:00
**Attendees:** Edoardo, Sara, Marco

## Agenda

1. Review current ranking results on the test vault
2. Decide on max-vs-average chunk scoring
3. Plan next steps for metadata filtering

## Discussion

### Chunk Scoring Strategy

Sara presented benchmark results comparing max-chunk and average-chunk ranking on a 500-note vault. Max-chunk ranking surfaced more relevant results in the top 5 for keyword-style queries, while average-chunk was marginally better for broad topic queries.

**Decision:** go with max-chunk similarity as the default. We can expose a flag later if users want average.

### Metadata Filtering

Marco proposed passing raw SQL WHERE clauses rather than inventing a custom filter syntax. Pros: zero new syntax to learn, full expressiveness. Cons: SQL injection risk if not parameterized, steeper learning curve for non-technical users.

**Decision:** accept raw SQL WHERE for v0.3, revisit with a safe subset or builder in v0.5.

## Action Items

- [ ] Edoardo: implement max-chunk ranking in `search` command
- [ ] Sara: add benchmark suite for ranking strategies
- [ ] Marco: draft the SQL filter documentation
- [ ] All: review updated spec by Friday
