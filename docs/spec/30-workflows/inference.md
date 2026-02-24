# Workflow: Field Inference

**Status: DRAFT**

**Cross-references:** [Terminology](../01-terminology.md) | [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md) | [Configuration: frontmatter.toml](../40-configuration/frontmatter-toml.md) | [Workflow: Init](init.md)

---

## Overview

Field inference takes a flat list of `(file_path, set_of_fields)` observations and produces, for each field, two sets of glob patterns:

- **`allowed`** — where the field *may* appear (at least one file has it)
- **`required`** — where the field *must* appear (all files have it)

These patterns are written to the TOML config (`mfv.toml` / `mdvs.toml`) during `init`. The user can then hand-edit them for finer control. `check` validates files against these patterns.

### Semantics

- `[]` = nowhere (field not expected anywhere)
- `["**"]` = everywhere (field applies to all files)
- `["blog/**"]` = all files at any depth under `blog/`
- `["blog/*"]` = files directly in `blog/`, not in subdirectories

Invariant: `required ⊆ allowed` — you cannot require a field where it's not allowed.

---

## Part 1: Theory

### Definitions

Given a vault with files $F = \{f_1, \ldots, f_n\}$, each file $f_i$ has a set of frontmatter fields $\text{fields}(f_i) \subseteq \mathcal{F}$ where $\mathcal{F}$ is the universe of all observed field names.

For a set of files $S \subseteq F$:

- $\text{all}(S) = \bigcap_{f \in S} \text{fields}(f)$ — fields present in **every** file
- $\text{any}(S) = \bigcup_{f \in S} \text{fields}(f)$ — fields present in **at least one** file
- Invariant: $\text{all}(S) \subseteq \text{any}(S)$

### The directory tree

Files live in a directory hierarchy. We model this as a tree:

- **Internal nodes** = directories. A directory is *always* an internal node, even if it contains files directly. Directories never appear as leaves.
- **Leaf nodes** = file-set aggregates. The set of files directly in a directory, represented as a single leaf. A directory with 3 files gets one leaf child holding the aggregate of those 3 files.
- Empty directories (no files, no non-empty subdirectories) are excluded.

This distinction is critical. A directory may have both a file-set leaf (its direct files) and subdirectory children. These are siblings in the tree, treated uniformly during merge and collapse.

#### Example tree

Given files: `blog/post1.md`, `blog/post2.md`, `blog/drafts/d1.md`, `notes/idea1.md`

```
root/
├── blog/                        (internal: directory)
│   ├── {post1, post2}           (leaf: file-set aggregate)
│   └── drafts/                  (internal: directory)
│       └── {d1}                 (leaf: file-set aggregate)
└── notes/                       (internal: directory)
    └── {idea1}                  (leaf: file-set aggregate)
```

### Computing `all` and `any`

At **leaf nodes**, `all` and `any` are computed directly from the files:

$$\text{all}(\text{leaf}) = \bigcap_{f \in \text{leaf.files}} \text{fields}(f)$$
$$\text{any}(\text{leaf}) = \bigcup_{f \in \text{leaf.files}} \text{fields}(f)$$

At **internal nodes** (directories), both sets are the intersection of children's corresponding sets:

$$\text{all}(\text{dir}) = \bigcap_{c \in \text{children}(\text{dir})} \text{all}(c)$$
$$\text{any}(\text{dir}) = \bigcap_{c \in \text{children}(\text{dir})} \text{any}(c)$$

Note: `any` at the parent is an **intersection**, not a union. A field is in `dir.any` only if **every** child has at least one file with that field. This means `dir.any` answers: "which fields appear in every branch of this subtree?"

### Glob pattern semantics: `*` vs `**`

The two glob depths map to two kinds of evidence:

| Pattern | Scope | Evidence source |
|---------|-------|-----------------|
| `dir/*` | Files directly in `dir/` | Leaf node (observed direct files only) |
| `dir/**` | All files at any depth under `dir/` | Directory node (aggregated full subtree) |

A **leaf node** has observed only the files directly in its directory. It has no information about subdirectories. Its natural scope is `*` — it can only vouch for what it has seen.

A **directory node**, after bottom-up merge, holds the aggregated picture of its entire subtree. When it confirms a field is in its `any` or `all`, that claim covers everything below it. Its natural scope is `**`.

This is not an exception or special case. It follows from what each node type represents:

- Leaf = observed direct files → `*`
- Directory = aggregated subtree → `**`
- Collapse upgrades `*` to `**` when a directory confirms the claim

### The collapse operation

Collapse works bottom-up, processing each directory node. For each field, the directory node's sets determine the action:

| Field in... | Action on `allowed` | Action on `required` |
|---|---|---|
| `dir.all` | Collapse: remove descendants, add `dir` | Collapse: remove descendants, add `dir` |
| `dir.any \ dir.all` | Collapse: remove descendants, add `dir` | No action |
| neither | No action | No action |

"Remove descendants" means: remove all entries whose path starts with `dir`'s path (component-wise, so `blog` does not match `blog-archive`). Then add `dir`'s path.

When collapse fires, a leaf's `*` contribution is removed and replaced by the directory's `**`. The upgrade is justified because the directory has aggregated evidence from the full subtree.

When collapse does *not* fire (field not in `dir.any`), the leaf's `*` stays. This is correct: sibling subtrees don't have the field, so `**` would be an unsupported claim.

### Initialization

- **`allowed`** is initialized from leaf nodes' `any` sets. Each field in `leaf.any` gets the leaf's directory path added to its allowed set.
- **`required`** is NOT initialized from leaves. It is only populated during collapse, from directory nodes' `all` sets.

Why not initialize `required` from leaves? Because a leaf's `all` set covers only direct files. A `required` pattern carries recursive implications — requiring a field at `dir/**` means every file under `dir/`, at any depth, must have it. Only the directory-level `all` (which reflects the full subtree intersection) has the evidence to support that claim.

### Converting paths to globs

After collapse, each field has a set of directory paths for `allowed` and `required`. Conversion to glob strings:

- Paths from leaf initialization (not collapsed) → `dir/*` (or `*` for root)
- Paths from collapse (directory nodes) → `dir/**` (or `**` for root)
- Root path `""` → `*` or `**` respectively

---

## Part 2: Worked Examples

### Example 1: Simple hierarchy

```
blog/post1.md  → {title, tags}
blog/post2.md  → {title}
blog/drafts/d1.md → {title, tags}
blog/drafts/d2.md → {title, tags}
notes/idea1.md → {title, tags}
notes/idea2.md → {title, tags}
papers/paper1.md → {title}
```

Tree after merge:

```
root/                          all: {title}        any: {title}
├── blog/                      all: {title}        any: {title, tags}
│   ├── {post1, post2}  [leaf] all: {title}        any: {title, tags}
│   └── drafts/                all: {title, tags}  any: {title, tags}
│       └── {d1, d2}    [leaf] all: {title, tags}  any: {title, tags}
├── notes/                     all: {title, tags}  any: {title, tags}
│   └── {idea1, idea2}  [leaf] all: {title, tags}  any: {title, tags}
└── papers/                    all: {title}        any: {title}
    └── {paper1}         [leaf] all: {title}        any: {title}
```

**title**: in `root.all` → collapse all the way up → `allowed = ["**"], required = ["**"]`

**tags**:
- Leaf init: allowed paths = {`blog/`, `blog/drafts/`, `notes/`}
- `drafts/` collapse: tags in `drafts.all` → collapse allowed + required under `drafts/`. Allowed: leaf `blog/drafts/` stays (it IS `drafts/`). Required: add `blog/drafts/`.
- `blog/` collapse: tags in `blog.any \ blog.all` → collapse allowed only. Remove `blog/` and `blog/drafts/` from allowed, add `blog/`. Required untouched.
- `notes/` collapse: tags in `notes.all` → collapse both. Allowed: `notes/` stays. Required: add `notes/`.
- `root/`: tags not in `root.any` → no action.
- Result: `allowed = ["blog/**", "notes/**"], required = ["blog/drafts/**", "notes/**"]`

### Example 2: Leaf next to subdirectory (the `*` vs `**` case)

```
a/b/file4.md      → {title, deep}
a/b/c/file3.md    → {title}
a/b/c/d/file1.md  → {title, deep}
a/b/c/d/file2.md  → {title, deep}
x/file5.md        → {title}
```

Tree after merge:

```
root/                          all: {title}  any: {title}
├── a/                         all: {title}  any: {title}
│   └── b/                    all: {title}  any: {title}
│       ├── {file4}     [leaf] all: {title, deep}  any: {title, deep}
│       └── c/                all: {title}  any: {title}
│           ├── {file3}  [leaf] all: {title}  any: {title}
│           └── d/            all: {title, deep}  any: {title, deep}
│               └── {f1,f2} [leaf] all: {title, deep}  any: {title, deep}
└── x/                        all: {title}  any: {title}
    └── {file5}        [leaf] all: {title}  any: {title}
```

**deep**:
- Leaf init: allowed paths = {`a/b/`, `a/b/c/d/`} (from leaf `any` sets)
- `d/` collapse: deep in `d.all` → collapse both. Allowed: `a/b/c/d/` stays. Required: add `a/b/c/d/`.
- `c/`: deep not in `c.any` → no action.
- `b/`: deep not in `b.any` (`b.any` = `{title,deep}` $\cap$ `{title}` = `{title}`) → no action.
- Leaf `a/b/` keeps its `*` pattern. It was never collapsed.
- Result: `allowed = ["a/b/*", "a/b/c/d/**"], required = ["a/b/c/d/**"]`

This is precise: `a/b/*` matches `file4.md` but not `a/b/c/file3.md`. If we had used `a/b/**`, it would incorrectly allow deep in `a/b/c/file3.md`.

### Example 3: Files at root alongside subdirectories

```
file6.md          → {title, deep}
a/b/c/d/file1.md  → {title, deep}
a/b/c/d/file2.md  → {title, deep}
a/b/c/file3.md    → {title}
a/b/file4.md      → {title}
x/file5.md        → {title}
```

**deep**:
- Leaf init: allowed paths = {`""` (root), `a/b/c/d/`}
- `d/` collapse: deep in `d.all` → `a/b/c/d/**`. Required: `a/b/c/d/**`.
- No other directory has deep in `any`.
- Root leaf `""` keeps its `*` pattern.
- Result: `allowed = ["*", "a/b/c/d/**"], required = ["a/b/c/d/**"]`

`*` at root matches `file6.md` but not files in subdirectories. Precise.

### Example 4: No ambiguity (field in its own subdirectory)

```
a/b/e/file4.md    → {title, deep}
a/b/c/file3.md    → {title}
a/b/c/d/file1.md  → {title, deep}
a/b/c/d/file2.md  → {title, deep}
x/file5.md        → {title}
```

**deep**:
- Leaf init: allowed paths = {`a/b/e/`, `a/b/c/d/`}
- `e/` collapse: deep in `e.all` → `a/b/e/**`. Required: `a/b/e/**`.
- `d/` collapse: deep in `d.all` → `a/b/c/d/**`. Required: `a/b/c/d/**`.
- No further collapse (deep not in `b.any` because `c.any` = {title}).
- Result: `allowed = ["a/b/c/d/**", "a/b/e/**"], required = ["a/b/c/d/**", "a/b/e/**"]`

No `*` needed — each occurrence of deep is in its own subdirectory. `**` is justified at both.

---

## Part 3: Algorithm

Three phases: **build**, **merge**, **collapse**.

### Phase 1: Build tree

Walk all file paths. For each file:

1. Extract the parent directory path.
2. Ensure the full directory chain exists in the tree (mkdir -p style). Each directory becomes an internal node.
3. Get or create the file-set leaf for that directory:
   - First file in a directory: create the leaf with `all = any = fields`.
   - Subsequent files: intersect `all`, union `any`.

Two maps track NodeIds:
- `dir_map: HashMap<PathBuf, NodeId>` — directory path → directory node
- `leaf_map: HashMap<PathBuf, NodeId>` — directory path → file-set leaf node

### Phase 2: Merge (bottom-up)

Traverse the tree bottom-up (via `NodeEdge::End` in post-order). For each internal node (has children):

```
node.all = intersect(child.all for each child)
node.any = intersect(child.any for each child)
```

Leaf nodes are skipped — they were populated during build.

### Phase 3: Collapse (bottom-up)

**Initialize:**

- For each leaf node, for each field in `leaf.any`: add the leaf's directory path to `allowed[field]`. Mark these entries as leaf-sourced.
- `required` starts empty.

**Collapse loop** (post-order, directory nodes only):

For each field at each directory node:

1. **Field in `node.all`**: collapse both `allowed` and `required` — remove descendant paths, add this node's path (marked as directory-sourced).
2. **Field in `node.any \ node.all`**: collapse `allowed` only — remove descendant paths, add this node's path (marked as directory-sourced).
3. **Field not in `node.any`**: skip (descendants keep their entries).

**Convert to globs:**

- Leaf-sourced paths → `dir/*` (or `*` for root)
- Directory-sourced paths → `dir/**` (or `**` for root)

---

## Part 4: Properties

### Correctness invariants

1. **`required ⊆ allowed`**: Required patterns are only added during collapse when `node.all` has the field, and `node.all ⊆ node.any`, so collapse also adds to allowed.
2. **No false negatives in `allowed`**: Every file that has a field is covered by at least one allowed pattern (initialized from the leaf that contains the file).
3. **No false positives in `required`**: A required pattern at `dir/**` means every file in the subtree has the field (because `node.all` is the intersection of all descendants' `all` sets).
4. **Deterministic output**: `BTreeMap` for fields, sorted glob vectors. Input order does not affect output.

### Conservative direction

The algorithm is conservative in the **permissive** direction for `allowed` — when uncertain, it allows more rather than less. Specifically:

- `*` patterns allow any file directly in a directory, even if only some observed files have the field.
- `**` patterns (from collapse) allow any file in the subtree, even at depths not yet observed.

This is intentional: `allowed` is a soft boundary. The user tightens it by hand-editing the config. `required`, by contrast, is strict — only emitted when there is full evidence.

### Monotonicity

Adding more files to the input can:
- Add new fields to the output.
- Expand `allowed` patterns (new locations observed).
- Shrink `required` patterns (a file without the field breaks the `all` chain).
- Never produce a contradiction with prior output (patterns only widen or narrow, never flip).

---

## Part 5: Implementation

Module: `crates/mdvs-schema/src/inference.rs`

### Public API

```rust
pub struct FieldPaths {
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

pub fn infer_field_paths(
    observations: &[(PathBuf, HashSet<String>)],
) -> BTreeMap<String, FieldPaths>
```

### Dependencies

- `indextree` (v4.7) — arena-allocated tree with `NodeId` handles and `traverse()` for post-order walking.

### Internal structure

- `NodeData { path, all, any }` — stored in `Arena<NodeData>`.
- `build_tree()` → `(Arena, NodeId)` — phase 1.
- `traverse_and_merge()` — phase 2, mutates arena in-place.
- `collapse()` → `BTreeMap<String, FieldPaths>` — phase 3, reads arena.
- `ensure_dir()` — creates directory chain, returns target NodeId.
- `collapse_paths()` — removes descendants via `Path::starts_with()`, adds ancestor.
- `paths_to_globs()` — converts `PathBuf` set to sorted glob strings.

### Key implementation patterns

**Collect-then-mutate**: `traverse()` borrows the arena immutably. To mutate nodes during post-order traversal, collect `NodeId`s into a `Vec` first, then iterate the vec and mutate.

**`Path::starts_with()`**: Component-based matching, not string prefix. `"blog"` does not start with `"blog-archive"`. This prevents false collapses on similar-looking paths.

### Implementation status

The current implementation does not yet distinguish `*` vs `**` — all patterns are emitted as `**`. The leaf-sourced vs directory-sourced tracking described in Part 3 needs to be added. See the "Leaf next to subdirectory" examples in Part 2 for the cases this affects.

---

## Related Documents

- [Terminology](../01-terminology.md) — definitions for field, vault
- [Workflow: Init](init.md) — inference runs during init
- [Configuration: frontmatter.toml](../40-configuration/frontmatter-toml.md) — output format
- [Crate: mdvs-schema](../10-crates/mdvs-schema/spec.md) — implementation home
