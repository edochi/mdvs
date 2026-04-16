# Concepts

mdvs has two layers — validation and search — each with its own set of concepts. These pages explain how things work under the hood.

- **[Types & Widening](./concepts/types.md)** — The type system, how types are inferred from values, and what happens when files disagree
- **[Schema Inference](./concepts/schema.md)** — How mdvs scans your directory and computes field paths, requirements, and constraints
- **[Validation](./concepts/validation.md)** — What `check` verifies, the five violation types, and how to read the output
- **[Constraints](./concepts/constraints.md)** — Categorical constraints, auto-inference heuristics, and manual overrides
- **[Search & Indexing](./concepts/search.md)** — Chunking, embeddings, incremental builds, and how results are ranked
