---
title: Inference service
status: active
owner: alice-chen
confidence: medium
last_reviewed: 2026-05-25
tags: [inference, retrieval, platform]
---

The serving layer in front of the retrieval stack — wraps the embedding
model, the ANN index, and the reranker behind a single endpoint. Active
rollout to internal callers; the retrieval team owns the SLO and the
on-call rotation.
