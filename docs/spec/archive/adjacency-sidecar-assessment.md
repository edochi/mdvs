# Columnar + CSR Sidecar: An Assessment for mdvs

Captured 2026-06-12. Not a decision — an architectural assessment for the case
of extending mdvs with relationship-aware queries.

## The Question

mdvs today is a columnar tabular store: one Lance dataset, one row per chunk,
frontmatter denormalized into a nested Struct, BM25 + (optional) IVF-PQ indexes
inside the dataset. This is well-suited to the queries the tool currently
advertises — semantic search, lexical search, SQL-style filters on typed
frontmatter, schema validation.

It is not well-suited to a class of queries that markdown corpora naturally
invite:

- **Backlinks.** "What files link to this one?" — requires scanning every
  chunk's body for `[[target]]` references.
- **Wikilink graph traversal.** "What's transitively reachable from this note by
  following wikilinks?" — currently impossible without a full scan + parse per
  query.
- **Reference integrity.** "Find all broken wikilinks across the vault." — full
  scan.
- **Frontmatter-declared relationships.** Fields like
  `related = ["[[a]]", "[[b]]"]` or `parent = "[[index]]"` are stored as opaque
  strings; they do not resolve to anything queryable.
- **Tag co-occurrence and shared-attribute lookups.** "Files that share
  `project = X` and `status = active`" works today via SQL on `data.*`, but
  "files connected to those via wikilinks" does not.
- **Path-scope crossed with relation-scope.** "Within `team/**`, find every file
  linked from any `meetings/**` note in the last quarter" composes path-scope
  rules (which mdvs already has) with edge filters (which it does not).

The wikilink currently lives in `chunk_text` after `[[target]]` → `target`
stripping for embedding purposes (`index/chunk.rs`). It is text. It is not an
edge.

The question this note assesses: if mdvs were to gain a first-class relationship
layer covering wikilinks, frontmatter references, and tag co-occurrence — _what
is the right on-disk storage strategy for that layer?_

## Storage Strategies Surveyed

Five real strategies exist for on-disk graph or graph-shaped data. They differ
on read performance, mutation cost, and integration burden.

### 1. Native graph storage (page-based)

Fixed-size node and edge records on disk, pointer-chained relationship lists per
node, separate property file, buffer pool for hot pages. Examples: Neo4j
(Community + Enterprise), Memgraph, TigerGraph.

- **Pros**: fastest possible traversal via pointer chasing; in-place mutation
  with redo log.
- **Cons**: bad cache locality for analytical scans (filter-and-scan workloads
  suffer); implementation is years of focused engineering. JVM stack in most
  cases. Not embeddable in a single Rust binary.

Not viable for mdvs.

### 2. LSM tree / KV store underneath

Encode triples as keys: `(src_id, edge_type, dst_id) → attributes`. Range scans
walk outgoing edges. The underlying KV store handles durability, compaction,
recovery. Examples: Dgraph (Badger), Cayley (BoltDB/LevelDB/RocksDB), Oxigraph
(RocksDB), SurrealDB (SurrealKV).

- **Pros**: mutation-friendly; durable; reuses battle-tested KV engines.
- **Cons**: traversal becomes multiple KV lookups (worse cache locality than
  native pointer-chasing); requires adding a second storage engine alongside
  Lance; relationship attributes need their own filter implementation.

Possible for mdvs but adds a parallel storage stack with its own compaction,
backup, and recovery semantics. The dependency footprint grows meaningfully.

### 3. Columnar tabular only (edges as a Lance table)

Add an `edges` Lance table parallel to `index`: rows are
`(source_file_id, target_file_id, relation_type, attrs, content_hash_at_creation)`.
Queries via DataFusion SQL with predicate pushdown. Examples: DuckDB + SQL/PGQ,
Apache AGE on PostgreSQL.

- **Pros**: stays within the existing storage stack; reuses Lance's columnar
  filter performance; one engine to operate.
- **Cons**: traversal beyond one hop becomes recursive CTEs, which DataFusion
  supports but does not optimize. For 1-hop "what links to this file" queries
  with attribute filters, this is actually excellent — Lance's columnar
  predicate pushdown shines. For multi-hop traversal, it degrades.

Viable for mdvs if the workload stays mostly 1-hop. It does not, in practice —
wikilink-based knowledge bases naturally invite transitive queries.

### 4. CSR / immutable adjacency sidecar

Compressed Sparse Row layout — `offsets[node] → start_in_neighbors[]` plus
`neighbors[]` packed contiguously — built once per segment, mmap'd, traversed
with pointer arithmetic. Mutation handled by append-only segments + periodic
compaction. Examples: WebGraph BV format (web-scale crawls compressed to GB),
Lucene + Tantivy inverted indexes (same architectural pattern for full-text),
FAISS HNSW indexes, LanceDB's own FTS index (already next to mdvs's data on
disk).

- **Pros**: fastest possible read-path traversal — pointer chase through mmap'd
  memory; startup is page-fault speed, no parse cost; only pages touched by the
  traversal are resident in RAM; well-precedented (every search engine on earth
  uses this shape).
- **Cons**: mutation requires either rebuild (cheap for small graphs) or
  append-only segments with periodic compaction (more code).

This is the pattern Lance itself uses internally for full-text search. It is not
exotic.

### 5. Edge list with B-tree indexes (relational fallback)

`(src, dst, type, attrs)` rows in SQLite or similar; B-tree on `src` and `dst`.
Works for small scale; degrades past low hundreds of thousands of edges. Not a
serious candidate alongside Lance.

## The Hybrid: Lance Tables + CSR Sidecar

The strategy that fits mdvs is the hybrid: keep Lance for what it already does
well (chunks, frontmatter, embeddings, BM25, scalar/vector indexes, SQL filters
on typed metadata) and add a CSR sidecar for adjacency. The data layout becomes:

```
.mdvs/
  index.lance/          — chunks + frontmatter + embeddings + FTS/vector indexes
  edges.lance/          — edge attributes: relation_type, source_file_id, target_file_id,
                         confidence, declared_at, content_hash_at_creation
  adjacency.mdix        — CSR sidecar: offsets[], neighbors[] (forward and reverse),
                         each neighbor entry points back to edges.lance row_id
```

Queries route by shape:

- **Attribute-filtered edge queries** ("all `parent` edges declared after X") →
  `edges.lance` via DataFusion, columnar pushdown, native Lance strength.
- **Pure traversal** ("what's k-hops from this file") → `adjacency.mdix` via
  mmap, native pointer-chase speed, no Lance involvement.
- **Filtered traversal** ("BFS from this file but only follow `related` edges")
  → sidecar walks topology, edge attributes resolve from `edges.lance` via
  row_id pointer on demand.
- **Composite** ("backlinks where the source file has `status = published`") →
  sidecar gives backlinks (1-hop reverse), join against `index.lance` for the
  source file's frontmatter.

The CSR sidecar is a binary file mdvs would own. The format is straightforward:

```
[header]                      magic, version, node_count, edge_count, segment_id
[forward.offsets]             length node_count + 1, u64
[forward.neighbors]           length edge_count, struct {target_node_id: u64, edge_row_id: u64}
[reverse.offsets]             length node_count + 1, u64
[reverse.neighbors]           length edge_count, struct {source_node_id: u64, edge_row_id: u64}
[footer]                      checksum
```

mmap'd on open. BFS or DFS is a tight loop walking
`forward.neighbors[forward.offsets[node]..forward.offsets[node+1]]`. The
`edge_row_id` lets any traversal step resolve to a Lance row for attribute
access without a scan.

## Why This Fits mdvs Specifically

The hybrid pattern is not chosen for theoretical reasons but for concrete
alignment with mdvs's existing shape.

**The read/write ratio matches.** mdvs is a CLI tool invoked from agent loops,
editors, and shell commands. Reads vastly outnumber writes. The sidecar's
optimization for read performance and weakness on per-write durability is
exactly the right tradeoff. Writes batch naturally: `mdvs build` and
`mdvs update` are the mutation points, both of which can rebuild or
append-compact the sidecar at the end of the operation.

**Startup latency matters.** mdvs already optimizes for "bare `mdvs search` does
everything in one shot." A graph layer that required parsing a JSON-or-similar
serialization on every invocation would defeat that property. The mmap sidecar
opens in page-fault time — milliseconds, not seconds, regardless of corpus size.
This is the same property Lance already gives mdvs for its columnar data.

**The stack stays Rust + Lance.** No additional storage engine, no FFI, no JVM,
no second compaction story to operate. The sidecar is a binary file mdvs writes
and reads with `memmap2` and a thin format module. The total dependency surface
for the graph layer is two crates (`memmap2`, possibly `zerocopy`) plus the
format code mdvs owns.

**The chunk identity layer already exists.** `file_id` is stable across
incremental builds (per `decisions.md`); `chunk_id` is per-chunk. A graph layer
that wants to address nodes can address either, depending on edge granularity.
The identity work mdvs has already done for incremental indexing is the same
identity work a graph layer needs.

**Path-scope rules compose.** mdvs's existing `[directory]` scoping (`team/**`
requires `role`) extends naturally to edge predicates: edge sources, targets, or
both can be filtered by path scope using Lance SQL filters on the source/target
file's `filepath` column. No new abstraction needed; the existing scoping reads
through to edge queries via the `edges.lance` join.

**The pattern is identical to what Lance does for FTS.** mdvs already operates a
Lucene-style architecture: tabular base + inverted index sidecar inside the
Lance dataset. Adding a CSR adjacency sidecar is the same architectural move
applied to a different access pattern. There is no philosophical drift in the
design.

**The dead-graph-DB market matters.** Both prominent embedded columnar graph
databases that would have been alternatives (KuzuDB, CozoDB) became unmaintained
within the past 12 months — KuzuDB archived October 2025, CozoDB no commits
since December 2024. Depending on an external embedded graph DB is now a
meaningful platform risk. Owning a small sidecar format that mdvs controls
eliminates that risk entirely.

## What This Enables

With the hybrid in place, the following queries become natural:

- `mdvs backlinks <file>` — list every file that links to the target via
  wikilink or frontmatter reference. 1-hop reverse traversal in the sidecar;
  ~microsecond response on any vault.
- `mdvs broken-links` — scan all neighbor entries whose target node has no
  corresponding file in `index.lance`. One pass over the sidecar, one anti-join.
- `mdvs related <file>` — k-hop neighborhood from the file with optional filters
  on edge type, relation tag, or target frontmatter (composing sidecar traversal
  with Lance row attribute access).
- `mdvs search "..." --connected-to <file>` — restrict semantic/lexical search
  results to files within k hops of an anchor file. Sidecar BFS produces the
  candidate set; the search runs over that filtered scope.
- `mdvs orphans` — files with neither inbound nor outbound edges.
  Reverse-and-forward scan over the sidecar; constant-memory traversal.
- `mdvs tag-graph` — derive an implicit graph from shared tag values,
  materialized as edges with `relation = "shares-tag:<tagname>"`. The sidecar
  handles the topology; tag values stay in `data.tags`.

None of these are answerable with the current columnar-only storage without a
full corpus scan.

## Tradeoffs Worth Naming

**Compaction is non-trivial.** Append-only segments degrade traversal
performance as their count grows (each traversal step walks all segments to find
a node's neighbors). The compaction strategy — when to fold N small segments
into one large one — has to be designed from day one, not bolted on later.
Lucene/Tantivy/RocksDB all live with this; the patterns are well-understood
(size-tiered, leveled, or time-windowed compaction). It is one to two weeks of
focused work to do correctly.

**Format ownership has long-tail cost.** mdvs would own a binary format with
versioning, backward-compatibility, and corruption-recovery concerns.
Manageable, but real. The mitigation is to keep the format simple (CSR is
roughly the simplest non-trivial graph format) and to gate format-version on the
dataset metadata so old sidecars are rejected cleanly on mismatch.

**Mutation is segment-rebuild, not in-place.** Updating an edge attribute does
not modify the sidecar; it modifies `edges.lance`. Adding or removing an edge
writes a new segment. This is correct for mdvs's workflow (mutation is batched
in `build`/`update`) but would be wrong for a daemon model with high write
throughput.

**Multi-hop pattern matching has no native dialect.** Cypher, SPARQL, and
SQL/PGQ all exist as standard query languages over graphs. The sidecar approach
does not provide one. Queries are written against the sidecar via a Rust API or,
for ad-hoc use, a small CLI surface (`mdvs neighbors`, `mdvs path`,
`mdvs subgraph`). For pattern matching beyond k-hop neighborhood and
shortest-path, the user would write code.

## Comparison Against the Alternatives

| Strategy                                     | Reads (1-hop filter) | Reads (k-hop traversal) | Mutation       | Integration        | Dependency risk     |
| -------------------------------------------- | -------------------- | ----------------------- | -------------- | ------------------ | ------------------- |
| Native graph (Neo4j-style)                   | excellent            | best                    | excellent      | wrong stack        | n/a                 |
| LSM/KV underneath                            | good                 | good                    | best           | new storage engine | RocksDB/sled stable |
| Columnar tabular only (edges in Lance)       | best                 | poor (recursive CTE)    | best           | minimal            | none                |
| **Columnar + CSR sidecar (this assessment)** | **best**             | **excellent**           | good (batched) | low (one format)   | none                |
| Edge list + B-tree (SQLite)                  | acceptable           | poor                    | good           | adds SQLite        | low                 |

The hybrid wins on every dimension that matches mdvs's workload shape, with the
single concession that mutation is batched rather than streaming. That
concession is free in mdvs's existing operational model.

## Open Questions Not Resolved Here

- Node granularity: are graph nodes files (`file_id`), chunks (`chunk_id`), or
  both? Most relationship queries are file-level, but heading-level wikilinks
  (`[[file#Section]]`) suggest chunk-level addressing for some edges.
- Edge derivation timing: are wikilink edges derived at `build` (alongside
  embedding) or in a separate `mdvs graph build` step? The former couples graph
  state to embedding state; the latter allows graph rebuilds without
  re-embedding.
- Frontmatter reference field declaration: how does the schema declare that a
  frontmatter field is a wikilink reference vs. a free string? Likely a new
  `[[fields.field]]` attribute (`is_reference = true`) parallel to existing
  constraints.
- Schema migration for existing vaults: extending `mdvs.toml` to declare
  relation extraction rules is a config schema change; the existing init/update
  flow needs to handle the new fields without breaking existing locks.

These are spec-level design questions, not storage-strategy questions. The
storage assessment above stands regardless of how they resolve.

## Conclusion

If mdvs adds a relationship layer, the right on-disk storage strategy is a
columnar tabular base (Lance, where the work already is) plus a mmap'd CSR
adjacency sidecar (one new binary format mdvs owns). The pattern is identical to
what Lance already does internally for full-text search. It composes naturally
with mdvs's existing identity layer, path scoping, and incremental build model.
It avoids the dependency risk of embedded graph databases (a category in active
decline) and the performance ceiling of pure columnar traversal via recursive
CTEs.

The work is bounded: format design, reader/writer modules, compaction logic, and
a thin query layer. Estimated two to three months for a credible V1, on top of
the existing storage stack, with no change to the validation layer.
