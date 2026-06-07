---
title: Embedding models
category: model
confidence: medium
last_reviewed: 2026-05-30
tags: [embeddings, model, cpu]
---

Models that map text to dense vectors capturing semantic similarity.
Two broad families: contextual transformers (E5, BGE, gte) and static
embeddings (Model2Vec / POTION) that distil a contextual model into a
lookup table for CPU-only inference. Static models trade a few points
of retrieval quality for orders-of-magnitude lower latency — the right
choice when the corpus is small and the deployment can't carry a GPU.
