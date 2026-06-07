---
title: RAG architectures
category: retrieval
confidence: low
last_reviewed: 2026-04-08
tags: [retrieval, llm, prompt]
---

Retrieval-augmented generation: the LLM answers from a chunk set
fetched at query time rather than from training-time memorisation.
Variants differ on what they retrieve (chunks, summaries, graphs)
and how they fuse with the prompt (concatenation, function-call,
agentic loop). The naive "embed everything, top-k cosine, stuff it in
the prompt" baseline is a starting point, not a finished system.
