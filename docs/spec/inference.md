# Inference

Deep-dive into the schema inference subsystem. For the module map see [architecture.md](./architecture.md).

Inference spans four file groups: type inference (`discover/infer/types.rs` + `discover/field_type.rs`), path inference (`discover/infer/paths.rs`), constraint inference (`discover/infer/constraints/`), and the orchestrator (`discover/infer/mod.rs`).

## Type Widening

`FieldType::from_widen(a, b)` at `discover/field_type.rs:29` computes the least upper bound of two types. The operation is symmetric: `widen(A, B) == widen(B, A)`.

| Type A | Type B | Result | Rule |
|--------|--------|--------|------|
| same | same | same | identity |
| Integer | Float | Float | numeric promotion |
| Array(T1) | Array(T2) | Array(widen(T1, T2)) | recursive |
| Object(K1) | Object(K2) | Object(merged) | union keys, widen shared |
| *anything else* | *anything else* | String | fallback (top type) |

Object merging: all keys from both sides are kept. Shared keys are widened recursively. Unique keys pass through unchanged.

The "anything else" fallback covers: Boolean+Integer, Boolean+Float, Boolean+String, scalar+Array, scalar+Object, Array+Object. All widen to String.

## Type Detection

`From<&Value> for FieldType` at `discover/field_type.rs:61` maps JSON values to types:

- `Number` with `is_i64() || is_u64()` → Integer, otherwise Float
- Empty array `[]` → Array(String) (placeholder)
- Non-empty array → fold elements with `from_widen()` to uniform type
- Object → recursive field-by-field inference
- Null → String (but see null transparency below)

## Null Transparency

In `infer_field_types()` at `discover/infer/types.rs:28`, null values are **transparent** in type widening:

1. **Skip in widening** — null values hit `continue` at line 45, never entering the `from_widen()` call. A field with `null` in file A and `42` in file B infers as Integer, not String.
2. **Track separately** — `nulls: HashSet<String>` records which fields had any null value. This becomes `nullable: true` on the `FieldTypeInfo`.
3. **Default for null-only** — fields appearing only as null (never a real value) default to String at line 66: `types.entry(key).or_insert(FieldType::String)`.

File presence is always tracked regardless of null — a null-valued field counts as "present" for allowed/required glob computation.

## Path Inference

The hardest algorithm in the codebase. Converts per-file field presence into glob patterns.

### Tree Construction

`DirectoryTree::from(scanned)` at `discover/infer/paths.rs:37` builds an arena-based tree:

1. Create root node (empty path)
2. For each file, extract the set of frontmatter field names
3. Ensure the file's parent directory exists as a node in the tree
4. Create or update a **leaf node** for each directory containing files:
   - `all` — intersection of field sets across all files in this directory
   - `any` — union of field sets across all files in this directory

### Bottom-up Merge

`merge()` at `discover/infer/paths.rs:241` propagates field presence up the tree via post-order traversal (`NodeEdge::End` = children processed before parent):

For each non-leaf node, aggregate children:
- `node.all` = intersection of all children's `all` sets (field present in every file under this subtree)
- `node.any` = intersection of all children's `any` sets

After merge, each internal node knows which fields appear in all descendants vs. some descendants.

### Glob Collapsing

`infer_paths()` at `discover/infer/paths.rs:171` converts the tree into glob patterns in three phases:

**Phase 1 — Seed from leaves.** For each leaf node, add every field in `any` as a shallow glob (`dir/*`):
```
GlobMap::insert_shallow(dir_path)  →  entries[dir] = Shallow
```

**Phase 2 — Collapse from parents.** Post-order walk over non-leaf nodes:
- Fields in `all` (present in every file under this subtree): collapse both `allowed` and `required` to recursive glob (`dir/**`)
- Fields in `any \ all` (some but not all files): collapse only `allowed`

```
GlobMap::collapse(ancestor_path):
    entries.retain(|p, _| !p.starts_with(ancestor_path))  // remove descendants
    entries[ancestor_path] = Recursive                      // replace with **
```

**Phase 3 — Emit globs.** `GlobMap::to_globs()` converts entries to sorted strings:
- `Shallow` → `dir/*` (or `*` for root)
- `Recursive` → `dir/**` (or `**` for root)

### Example

Given files: `blog/a.md` (title, tags), `blog/b.md` (title), `notes/c.md` (title, tags):

- Leaf `blog/`: all={title}, any={title, tags}
- Leaf `notes/`: all={title, tags}, any={title, tags}
- After merge, root: all={title}, any={title, tags}
- For `title`: root.all → collapse to `**` in both allowed and required
- For `tags`: root.any\all → collapse allowed to `**`; blog.any has tags with Shallow initially, notes.all has tags → required gets `notes/**`

Result: `title` allowed=`["**"]` required=`["**"]`; `tags` allowed=`["**"]` required=`["notes/**"]`.

## Categorical Inference

`infer_constraints()` at `discover/infer/constraints/mod.rs:13` applies a heuristic after type+path inference:

1. **Type check** — field must be String, Integer, Array(String), or Array(Integer)
2. **Distinct cap** — `distinct_values.len() <= max_categories` (default 10)
3. **Repetition** — `occurrence_count / distinct_values.len() >= min_repetition` (default 2)

Distinct values and occurrence counts are collected during type inference in `collect_distinct_values()` at `discover/infer/types.rs:86`. For arrays, counting is element-level (each array element is one occurrence). Null values are excluded.

Values are converted from `serde_json::Value` to `toml::Value` (String→String, Number→Integer) and sorted for deterministic output.

## Orchestration

`InferredSchema::infer(scanned)` at `discover/infer/mod.rs:78` runs three phases sequentially:

1. **Type phase** — `infer_field_types(scanned)` → `BTreeMap<String, FieldTypeInfo>` (widened types, file lists, distinct values, occurrence counts, nullable flags)
2. **Path phase** — `DirectoryTree::from(scanned)` → `tree.infer_paths()` → `BTreeMap<String, FieldPaths>` (allowed + required globs)
3. **Merge** — combine type info and path info into `Vec<InferredField>`, sorted by name

The categorical heuristic is NOT applied here — it runs later in `from_inferred()` (`schema/config.rs`) or `update::run()` when converting `InferredField` to `TomlField`.
