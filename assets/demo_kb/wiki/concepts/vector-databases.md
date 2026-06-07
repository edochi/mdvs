---
title: Vector databases
category: infra
confidence: high
last_reviewed: 2026-05-22
tags: [retrieval, embeddings, search]
---

Specialised storage for **dense vectors** and approximate-nearest-neighbour
queries. The retrieval primitive under semantic search and RAG. Common
choices: `LanceDB` (file-based, columnar, ANN built in), `pgvector`
(Postgres extension, exact + IVFFlat), and `Qdrant` (server with payload
filtering). **Filtering by structured metadata** alongside the vector
similarity is the operation that distinguishes them in practice.
