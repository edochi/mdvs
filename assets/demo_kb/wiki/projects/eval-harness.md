---
title: Eval harness
status: shipped
owner: carol-park
confidence: high
last_reviewed: 2026-06-01
tags: [eval, model, retrieval]
---

Internal benchmark harness for embedding-model regression testing.
Runs the standard retrieval benchmarks (BEIR, MTEB subset) plus an
in-house golden set; emits a single recall@10 number per model
checkpoint. Required gate before any model swap.
