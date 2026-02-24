//! Tree-based inference of `allowed` and `required` glob patterns from file observations.
//!
//! Given a flat list of `(file_path, field_names)` pairs, builds a directory tree,
//! computes field presence sets bottom-up, and collapses into glob patterns.
//!
//! Tree structure:
//! - **Internal nodes** = directories (never leaves, even if they contain files)
//! - **Leaf nodes** = file-set aggregates (the set of files directly in a directory)
//! - A directory with files gets an explicit file-set leaf child

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use indextree::{Arena, NodeEdge, NodeId};

/// Whether a glob pattern covers direct children only (`*`) or any depth (`**`).
///
/// Leaf nodes (file-set aggregates) produce `Shallow` patterns because they only
/// observed direct files. Directory nodes produce `Recursive` patterns via collapse
/// because they have aggregated evidence from the full subtree.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobDepth {
    /// `*` — direct children only (from leaf initialization).
    Shallow,
    /// `**` — any depth (from directory collapse).
    Recursive,
}

/// Per-field inferred glob patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPaths {
    /// Glob patterns where this field may appear (at least one file has it in every subtree).
    pub allowed: Vec<String>,
    /// Glob patterns where this field must appear (all files have it).
    pub required: Vec<String>,
}

/// Data stored at each node of the directory tree.
struct NodeData {
    /// Full path from root (empty for root, e.g. "blog", "blog/posts").
    /// For file-set leaves, this is the parent directory's path.
    path: PathBuf,
    /// Fields present in ALL files at this node.
    all: HashSet<String>,
    /// Fields present in at least ONE file at this node.
    any: HashSet<String>,
}

/// Infer `allowed` and `required` glob patterns for each field from file observations.
///
/// Input: slice of `(file_path, field_names)` pairs where paths are relative to the vault root.
/// Output: sorted map from field name to its inferred glob patterns.
///
/// Empty input returns an empty map.
pub fn infer_field_paths(
    observations: &[(PathBuf, HashSet<String>)],
) -> BTreeMap<String, FieldPaths> {
    if observations.is_empty() {
        return BTreeMap::new();
    }
    let (mut arena, root) = build_tree(observations);
    traverse_and_merge(&mut arena, root);
    collapse(&arena, root)
}

/// Phase 1: Build the directory tree and populate file-set leaf nodes.
///
/// For each file, ensures the full directory chain exists (mkdir -p style)
/// and creates/updates a file-set leaf child under the parent directory.
fn build_tree(observations: &[(PathBuf, HashSet<String>)]) -> (Arena<NodeData>, NodeId) {
    let mut arena = Arena::new();
    let root = arena.new_node(NodeData {
        path: PathBuf::new(),
        all: HashSet::new(),
        any: HashSet::new(),
    });

    // Map directory path → directory NodeId.
    let mut dir_map: HashMap<PathBuf, NodeId> = HashMap::new();
    dir_map.insert(PathBuf::new(), root);

    // Map directory path → file-set leaf NodeId (created on first file in that dir).
    let mut leaf_map: HashMap<PathBuf, NodeId> = HashMap::new();

    for (file_path, fields) in observations {
        let parent_dir = file_path.parent().unwrap_or(Path::new("")).to_path_buf();

        // Ensure the full directory chain exists.
        let dir_node_id = ensure_dir(&mut arena, &mut dir_map, root, &parent_dir);

        // Get or create the file-set leaf for this directory.
        let leaf_id = *leaf_map.entry(parent_dir.clone()).or_insert_with(|| {
            let leaf = arena.new_node(NodeData {
                path: parent_dir,
                all: fields.clone(),
                any: fields.clone(),
            });
            dir_node_id.append(leaf, &mut arena);
            leaf
        });

        // Update the file-set leaf: intersect `all`, union `any`.
        // (First file already set all=any=fields via or_insert_with above.)
        let node = arena[leaf_id].get_mut();
        node.all = node.all.intersection(fields).cloned().collect();
        node.any = node.any.union(fields).cloned().collect();
    }

    (arena, root)
}

/// Ensure a directory path exists in the tree, creating intermediate nodes as needed.
/// Returns the NodeId of the target directory.
fn ensure_dir(
    arena: &mut Arena<NodeData>,
    dir_map: &mut HashMap<PathBuf, NodeId>,
    root: NodeId,
    dir_path: &Path,
) -> NodeId {
    if let Some(&id) = dir_map.get(dir_path) {
        return id;
    }

    // Collect the chain of ancestors that need creating.
    let mut to_create = Vec::new();
    let mut current = dir_path.to_path_buf();
    while !dir_map.contains_key(&current) {
        to_create.push(current.clone());
        current = current.parent().unwrap_or(Path::new("")).to_path_buf();
    }

    // Create from shallowest to deepest.
    to_create.reverse();
    for path in to_create {
        let parent_path = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let parent_id = *dir_map.get(&parent_path).unwrap_or(&root);

        let new_node = arena.new_node(NodeData {
            path: path.clone(),
            all: HashSet::new(),
            any: HashSet::new(),
        });
        parent_id.append(new_node, arena);
        dir_map.insert(path, new_node);
    }

    dir_map[dir_path]
}

/// Phase 2: Bottom-up traversal to compute parent `all`/`any` sets from children.
///
/// Uses `NodeEdge::End` events from `traverse()` to guarantee children are processed
/// before parents. Collects NodeIds first to avoid borrow conflict.
fn traverse_and_merge(arena: &mut Arena<NodeData>, root: NodeId) {
    let post_order: Vec<NodeId> = root
        .traverse(arena)
        .filter_map(|edge| match edge {
            NodeEdge::End(id) => Some(id),
            _ => None,
        })
        .collect();

    for node_id in post_order {
        if arena[node_id].first_child().is_none() {
            // Leaf node — already populated by build_tree.
            continue;
        }

        // Gather children's all/any sets.
        let mut child_sets: Vec<(HashSet<String>, HashSet<String>)> = Vec::new();
        let mut child_id = arena[node_id].first_child();
        while let Some(cid) = child_id {
            let child = arena[cid].get();
            child_sets.push((child.all.clone(), child.any.clone()));
            child_id = arena[cid].next_sibling();
        }

        // Intersect all children's sets.
        let merged_all = intersect_all(child_sets.iter().map(|(a, _)| a));
        let merged_any = intersect_all(child_sets.iter().map(|(_, a)| a));

        let node = arena[node_id].get_mut();
        node.all = merged_all;
        node.any = merged_any;
    }
}

/// Intersect an iterator of HashSets. Empty iterator returns an empty set.
fn intersect_all<'a>(mut sets: impl Iterator<Item = &'a HashSet<String>>) -> HashSet<String> {
    let Some(first) = sets.next() else {
        return HashSet::new();
    };
    let mut result = first.clone();
    for set in sets {
        result = result.intersection(set).cloned().collect();
    }
    result
}

/// Phase 3: Build the result map by initializing from leaves and collapsing bottom-up.
fn collapse(arena: &Arena<NodeData>, root: NodeId) -> BTreeMap<String, FieldPaths> {
    // Per-field tracking: directory paths with their glob depth for allowed/required.
    let mut allowed: HashMap<String, HashMap<PathBuf, GlobDepth>> = HashMap::new();
    let mut required: HashMap<String, HashMap<PathBuf, GlobDepth>> = HashMap::new();

    // Initialize `allowed` from file-set leaf nodes with Shallow depth.
    // Leaves only observed direct files, so `*` is the honest scope.
    // `required` is NOT initialized here — it comes from the collapse step only,
    // where directory `all` sets reflect the full subtree (file-set + subdirectories).
    for node_id in root.descendants(arena) {
        if arena[node_id].first_child().is_some() {
            continue;
        }
        let node = arena[node_id].get();
        for field in &node.any {
            allowed
                .entry(field.clone())
                .or_default()
                .insert(node.path.clone(), GlobDepth::Shallow);
        }
    }

    // Bottom-up collapse: process internal (directory) nodes only.
    // Collapse replaces descendant entries with a single Recursive entry.
    let post_order: Vec<NodeId> = root
        .traverse(arena)
        .filter_map(|edge| match edge {
            NodeEdge::End(id) => Some(id),
            _ => None,
        })
        .collect();

    for node_id in post_order {
        if arena[node_id].first_child().is_none() {
            continue;
        }

        let node = arena[node_id].get();
        let node_path = &node.path;

        // Fields in `all` → collapse allowed, and add to required (with collapse).
        for field in &node.all {
            collapse_paths(allowed.entry(field.clone()).or_default(), node_path);
            collapse_paths(required.entry(field.clone()).or_default(), node_path);
        }

        // Fields in `any \ all` → collapse only allowed.
        for field in node.any.difference(&node.all) {
            collapse_paths(allowed.entry(field.clone()).or_default(), node_path);
        }
    }

    // Convert to output format.
    let all_fields: HashSet<&String> = allowed.keys().chain(required.keys()).collect();
    let mut result = BTreeMap::new();
    for field in all_fields {
        let a = allowed.get(field).map(paths_to_globs).unwrap_or_default();
        let r = required.get(field).map(paths_to_globs).unwrap_or_default();
        result.insert(
            field.clone(),
            FieldPaths {
                allowed: a,
                required: r,
            },
        );
    }
    result
}

/// Remove descendant entries and add the ancestor path as `Recursive`.
///
/// "Descendant" = any path that starts with `ancestor_path` (using `Path::starts_with`,
/// which operates component-by-component). This upgrades any `Shallow` leaf entries
/// to the directory's `Recursive` scope when the directory has evidence for the field.
fn collapse_paths(paths: &mut HashMap<PathBuf, GlobDepth>, ancestor_path: &Path) {
    paths.retain(|p, _| !p.starts_with(ancestor_path));
    paths.insert(ancestor_path.to_path_buf(), GlobDepth::Recursive);
}

/// Convert a map of directory paths + depths to sorted glob strings.
///
/// `Shallow` → `dir/*` (or `*` for root), `Recursive` → `dir/**` (or `**` for root).
fn paths_to_globs(paths: &HashMap<PathBuf, GlobDepth>) -> Vec<String> {
    let mut globs: Vec<String> = paths
        .iter()
        .map(|(p, depth)| {
            let suffix = match depth {
                GlobDepth::Shallow => "*",
                GlobDepth::Recursive => "**",
            };
            if p.as_os_str().is_empty() {
                suffix.to_string()
            } else {
                format!("{}/{suffix}", p.display())
            }
        })
        .collect();
    globs.sort();
    globs
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: build an observation entry.
    fn obs(path: &str, fields: &[&str]) -> (PathBuf, HashSet<String>) {
        (
            PathBuf::from(path),
            fields.iter().map(|s| s.to_string()).collect(),
        )
    }

    /// Helper: build expected FieldPaths.
    fn fp(allowed: &[&str], required: &[&str]) -> FieldPaths {
        FieldPaths {
            allowed: allowed.iter().map(|s| s.to_string()).collect(),
            required: required.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn empty_input() {
        let result = infer_field_paths(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn single_file_at_root() {
        let result = infer_field_paths(&[obs("a.md", &["title", "tags"])]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["tags"], fp(&["**"], &["**"]));
    }

    #[test]
    fn root_only_files_partial() {
        // All files at root, `title` in all, `tags` and `date` in some.
        let result = infer_field_paths(&[
            obs("a.md", &["title", "tags"]),
            obs("b.md", &["title"]),
            obs("c.md", &["title", "date"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["tags"], fp(&["**"], &[]));
        assert_eq!(result["date"], fp(&["**"], &[]));
    }

    #[test]
    fn single_directory_flat() {
        // All files in blog/, title in all, tags/date in some.
        // Single child of root → everything collapses to root (**).
        let result = infer_field_paths(&[
            obs("blog/a.md", &["title", "tags", "date"]),
            obs("blog/b.md", &["title", "date"]),
            obs("blog/c.md", &["title", "tags"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["tags"], fp(&["**"], &[]));
        assert_eq!(result["date"], fp(&["**"], &[]));
    }

    #[test]
    fn two_directories_shared_and_unique_fields() {
        let result = infer_field_paths(&[
            obs("blog/a.md", &["title", "tags", "date"]),
            obs("blog/b.md", &["title", "date"]),
            obs("papers/x.md", &["title", "doi"]),
            obs("papers/y.md", &["title", "doi", "date"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["date"], fp(&["**"], &["blog/**"]));
        assert_eq!(result["tags"], fp(&["blog/**"], &[]));
        assert_eq!(result["doi"], fp(&["papers/**"], &["papers/**"]));
    }

    #[test]
    fn deep_nesting_partial_collapse() {
        let result = infer_field_paths(&[
            obs("blog/posts/a.md", &["title", "tags"]),
            obs("blog/posts/b.md", &["title", "tags"]),
            obs("blog/drafts/c.md", &["title", "draft"]),
            obs("papers/x.md", &["title", "doi"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["tags"], fp(&["blog/posts/**"], &["blog/posts/**"]));
        assert_eq!(
            result["draft"],
            fp(&["blog/drafts/**"], &["blog/drafts/**"])
        );
        assert_eq!(result["doi"], fp(&["papers/**"], &["papers/**"]));
    }

    #[test]
    fn worked_example_from_design_doc() {
        // Matches the example in inference.md.
        let result = infer_field_paths(&[
            obs("blog/post1.md", &["title", "tags"]),
            obs("blog/post2.md", &["title"]),
            obs("blog/drafts/d1.md", &["title", "tags"]),
            obs("blog/drafts/d2.md", &["title", "tags"]),
            obs("notes/idea1.md", &["title", "tags"]),
            obs("notes/idea2.md", &["title", "tags"]),
            obs("papers/paper1.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(
            result["tags"],
            fp(&["blog/**", "notes/**"], &["blog/drafts/**", "notes/**"])
        );
    }

    #[test]
    fn mixed_root_and_subdirectory() {
        // Files at root AND in a subdirectory.
        // Root dir has two children: root file-set leaf and blog/ dir.
        let result = infer_field_paths(&[
            obs("a.md", &["title", "draft"]),
            obs("blog/b.md", &["title", "tags"]),
        ]);
        // root file-set leaf: all={title,draft}, any={title,draft}
        // blog file-set leaf: all={title,tags}, any={title,tags}
        // blog dir: all={title,tags}, any={title,tags}
        // root dir: intersect(root-file-set, blog-dir) = all={title}, any={title}
        // title: in root.all → ["**"]/["**"]
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        // draft: in root-file-set any → allowed=["*"] (leaf, shallow). Not in root.any → no collapse.
        assert_eq!(result["draft"], fp(&["*"], &[]));
        // tags: not in root.any → stays at blog.
        assert_eq!(result["tags"], fp(&["blog/**"], &["blog/**"]));
    }

    #[test]
    fn single_file_in_subdirectory() {
        let result = infer_field_paths(&[obs("deep/nested/dir/a.md", &["title"])]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
    }

    #[test]
    fn field_in_every_file_across_many_dirs() {
        let result = infer_field_paths(&[
            obs("a/x.md", &["title", "extra_a"]),
            obs("b/y.md", &["title", "extra_b"]),
            obs("c/z.md", &["title", "extra_c"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["extra_a"], fp(&["a/**"], &["a/**"]));
        assert_eq!(result["extra_b"], fp(&["b/**"], &["b/**"]));
        assert_eq!(result["extra_c"], fp(&["c/**"], &["c/**"]));
    }

    // --- Edge cases ---

    #[test]
    fn file_with_empty_field_set() {
        let result = infer_field_paths(&[obs("a.md", &["title"]), obs("b.md", &[])]);
        assert_eq!(result["title"], fp(&["**"], &[]));
    }

    #[test]
    fn all_files_empty_fields() {
        let result = infer_field_paths(&[obs("a.md", &[]), obs("blog/b.md", &[])]);
        assert!(result.is_empty());
    }

    #[test]
    fn completely_disjoint_fields_same_directory() {
        let result = infer_field_paths(&[
            obs("notes/a.md", &["alpha", "beta"]),
            obs("notes/b.md", &["gamma", "delta"]),
        ]);
        assert_eq!(result["alpha"], fp(&["**"], &[]));
        assert_eq!(result["beta"], fp(&["**"], &[]));
        assert_eq!(result["gamma"], fp(&["**"], &[]));
        assert_eq!(result["delta"], fp(&["**"], &[]));
    }

    #[test]
    fn completely_disjoint_fields_different_directories() {
        let result = infer_field_paths(&[
            obs("a/x.md", &["alpha"]),
            obs("b/y.md", &["beta"]),
        ]);
        assert_eq!(result["alpha"], fp(&["a/**"], &["a/**"]));
        assert_eq!(result["beta"], fp(&["b/**"], &["b/**"]));
    }

    #[test]
    fn field_in_exactly_one_file_among_many() {
        let result = infer_field_paths(&[
            obs("a/1.md", &["title"]),
            obs("a/2.md", &["title"]),
            obs("a/3.md", &["title"]),
            obs("a/4.md", &["title", "rare"]),
            obs("b/5.md", &["title"]),
        ]);
        assert_eq!(result["rare"], fp(&["a/**"], &[]));
        assert_eq!(result["title"], fp(&["**"], &["**"]));
    }

    #[test]
    fn similar_path_prefixes_no_false_collapse() {
        let result = infer_field_paths(&[
            obs("blog/a.md", &["title", "tags"]),
            obs("blog-archive/b.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["tags"], fp(&["blog/**"], &["blog/**"]));
    }

    #[test]
    fn four_level_deep_nesting() {
        let result = infer_field_paths(&[
            obs("a/b/c/d/file1.md", &["title", "deep"]),
            obs("a/b/c/d/file2.md", &["title", "deep"]),
            obs("a/b/c/file3.md", &["title"]),
            obs("a/b/file4.md", &["title"]),
            obs("x/file5.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(result["deep"], fp(&["a/b/c/d/**"], &["a/b/c/d/**"]));
    }

    #[test]
    fn intermediate_collapse_stops_at_right_level() {
        let result = infer_field_paths(&[
            obs("blog/posts/a.md", &["title", "tags"]),
            obs("blog/posts/b.md", &["title", "tags"]),
            obs("blog/reviews/c.md", &["title", "tags"]),
            obs("blog/reviews/d.md", &["title", "tags"]),
            obs("blog/meta/e.md", &["title"]),
            obs("other/f.md", &["title"]),
        ]);
        assert_eq!(
            result["tags"],
            fp(
                &["blog/posts/**", "blog/reviews/**"],
                &["blog/posts/**", "blog/reviews/**"]
            )
        );
    }

    #[test]
    fn input_order_does_not_affect_output() {
        let obs_a = vec![
            obs("blog/a.md", &["title", "tags"]),
            obs("blog/b.md", &["title"]),
            obs("notes/c.md", &["title", "tags"]),
        ];
        let obs_b = vec![
            obs("notes/c.md", &["title", "tags"]),
            obs("blog/b.md", &["title"]),
            obs("blog/a.md", &["title", "tags"]),
        ];
        assert_eq!(infer_field_paths(&obs_a), infer_field_paths(&obs_b));
    }

    #[test]
    fn single_file_with_many_fields() {
        let result = infer_field_paths(&[obs("x.md", &["a", "b", "c", "d", "e"])]);
        for field in &["a", "b", "c", "d", "e"] {
            assert_eq!(result[*field], fp(&["**"], &["**"]));
        }
    }

    #[test]
    fn empty_fields_alongside_populated() {
        let result = infer_field_paths(&[
            obs("blog/a.md", &["title"]),
            obs("blog/b.md", &[]),
            obs("notes/c.md", &["title"]),
            obs("notes/d.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["notes/**"]));
    }

    #[test]
    fn mixed_root_and_subdir_partial_overlap() {
        let result = infer_field_paths(&[
            obs("a.md", &["title", "draft"]),
            obs("b.md", &["title"]),
            obs("blog/c.md", &["title", "draft"]),
            obs("blog/d.md", &["title", "tags"]),
        ]);
        // root file-set: all={title} (a has draft, b doesn't), any={title,draft}
        // blog file-set: all={title} (c has draft, d has tags), any={title,draft,tags}
        // blog dir: all={title}, any={title,draft,tags}
        // root dir: all={title}, any={title,draft} (intersect {title,draft} ∩ {title,draft,tags})
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        // draft: in root.any \ root.all → allowed collapses to root (Recursive), required=[]
        assert_eq!(result["draft"], fp(&["**"], &[]));
        // tags: only in blog.any, not in root.any → stays at blog
        assert_eq!(result["tags"], fp(&["blog/**"], &[]));
    }

    // --- * vs ** depth tests ---

    #[test]
    fn leaf_next_to_subdirectory() {
        // file4.md sits directly in a/b/ alongside subdirectory c/.
        // deep is in the leaf but not in all of c/'s subtree → no collapse at b/.
        // The leaf keeps its * pattern.
        let result = infer_field_paths(&[
            obs("a/b/file4.md", &["title", "deep"]),
            obs("a/b/c/file3.md", &["title"]),
            obs("a/b/c/d/file1.md", &["title", "deep"]),
            obs("a/b/c/d/file2.md", &["title", "deep"]),
            obs("x/file5.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        // a/b/* (leaf, shallow) + a/b/c/d/** (collapsed by d/)
        assert_eq!(
            result["deep"],
            fp(&["a/b/*", "a/b/c/d/**"], &["a/b/c/d/**"])
        );
    }

    #[test]
    fn root_files_with_deep_subdirectory() {
        // file6.md at root has deep, a/b/c/d/ has deep, nothing in between.
        // Root leaf keeps * because root.any doesn't include deep.
        let result = infer_field_paths(&[
            obs("file6.md", &["title", "deep"]),
            obs("a/b/c/d/file1.md", &["title", "deep"]),
            obs("a/b/c/d/file2.md", &["title", "deep"]),
            obs("a/b/c/file3.md", &["title"]),
            obs("a/b/file4.md", &["title"]),
            obs("x/file5.md", &["title"]),
        ]);
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        // * (root leaf, shallow) + a/b/c/d/** (collapsed by d/)
        assert_eq!(
            result["deep"],
            fp(&["*", "a/b/c/d/**"], &["a/b/c/d/**"])
        );
    }

    #[test]
    fn leaf_alone_no_sibling_dirs() {
        // When a leaf is the only child, its parent directory collapses it to **.
        // This confirms * doesn't leak into output when collapse fires.
        let result = infer_field_paths(&[
            obs("notes/a.md", &["title", "tags"]),
            obs("notes/b.md", &["title", "tags"]),
            obs("blog/c.md", &["title"]),
        ]);
        // tags: notes leaf has tags in all. notes/ collapses → **.
        // Not in root.any (blog doesn't have tags) → stays at notes.
        assert_eq!(result["tags"], fp(&["notes/**"], &["notes/**"]));
    }

    // --- Complex scenarios ---

    #[test]
    fn star_patterns_at_multiple_levels() {
        // x appears at root leaf, at a/ leaf, and deep in a/b/c/d/e/.
        // Intermediate levels (a/b/, a/b/c/, a/b/c/d/) don't have x.
        // Should produce * at two levels and ** at the deepest.
        let result = infer_field_paths(&[
            obs("root.md", &["title", "x"]),
            obs("a/file.md", &["title", "x"]),
            obs("a/b/file.md", &["title"]),
            obs("a/b/c/d/e/f1.md", &["title", "x"]),
            obs("a/b/c/d/e/f2.md", &["title", "x"]),
            obs("other/f.md", &["title"]),
        ]);
        // Tree after merge:
        //   root:   any={title} (root-leaf.any ∩ a/.any ∩ other/.any = {t,x}∩{t}∩{t})
        //   a/:     any={title} (a-leaf.any ∩ b/.any = {t,x}∩{t})
        //   b/:     any={title} (b-leaf.any ∩ c/.any = {t}∩{t,x} = {t})
        //   c/→d/→e/: single-child chain, all have all={t,x}. Collapse absorbs up to c/.
        // Root leaf and a/ leaf keep * (uncollapsed).
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(
            result["x"],
            fp(&["*", "a/*", "a/b/c/**"], &["a/b/c/**"])
        );
    }

    #[test]
    fn alternating_presence_across_five_levels() {
        // x appears at levels 1, 3, 5 but NOT at levels 2, 4.
        // Each appearance is separated by a gap, preventing any collapse
        // except at the deepest single-child chain (level 5).
        let result = infer_field_paths(&[
            obs("a/f1.md", &["title", "x"]),
            obs("a/b/f2.md", &["title"]),
            obs("a/b/c/f3.md", &["title", "x"]),
            obs("a/b/c/d/f4.md", &["title"]),
            obs("a/b/c/d/e/f5.md", &["title", "x"]),
        ]);
        // e/: single child leaf with x in all → collapse to **
        // d/: children = d-leaf(no x) + e/(x in any). d.any = {t}∩{t,x} = {t}. No collapse.
        // c/: children = c-leaf(x) + d/(no x in any). c.any = {t,x}∩{t} = {t}. No collapse.
        // b/: children = b-leaf(no x) + c/(no x in any). No collapse.
        // a/: children = a-leaf(x) + b/(no x). a.any = {t,x}∩{t} = {t}. No collapse.
        // root: single child a/. root.any = {t}. No collapse.
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        assert_eq!(
            result["x"],
            fp(&["a/*", "a/b/c/*", "a/b/c/d/e/**"], &["a/b/c/d/e/**"])
        );
    }

    #[test]
    fn leaf_and_subtree_both_have_field_everywhere() {
        // Files directly in dir/ AND in dir/sub/ ALL have x.
        // dir.all includes x → collapses leaf * and sub ** into dir/**.
        let result = infer_field_paths(&[
            obs("dir/a.md", &["title", "x"]),
            obs("dir/b.md", &["title", "x"]),
            obs("dir/sub/c.md", &["title", "x"]),
            obs("dir/sub/d.md", &["title", "x"]),
            obs("other/e.md", &["title"]),
        ]);
        // dir leaf: all={t,x}. dir/sub leaf: all={t,x}. sub/: all={t,x}.
        // dir/: all = {t,x}∩{t,x} = {t,x}. x in dir.all → collapse both.
        // root.any = {t,x}∩{t} = {t}. No further collapse.
        assert_eq!(result["x"], fp(&["dir/**"], &["dir/**"]));
    }

    #[test]
    fn leaf_and_subtree_partially_share_field() {
        // Files directly in dir/ ALL have x, but dir/sub/ only partially has x.
        // dir.any has x (both children have at least one) → collapse allowed to **.
        // dir.all doesn't have x (sub.all lacks x) → required stays empty.
        let result = infer_field_paths(&[
            obs("dir/a.md", &["title", "x"]),
            obs("dir/b.md", &["title", "x"]),
            obs("dir/sub/c.md", &["title", "x"]),
            obs("dir/sub/d.md", &["title"]),
            obs("other/e.md", &["title"]),
        ]);
        // dir leaf: all={t,x}, any={t,x}.
        // sub leaf: all={t}, any={t,x}. sub/: all={t}, any={t,x}.
        // dir/: all = {t,x}∩{t} = {t}. any = {t,x}∩{t,x} = {t,x}.
        // x in dir.any \ dir.all → collapse allowed only → Recursive.
        assert_eq!(result["x"], fp(&["dir/**"], &[]));
    }

    #[test]
    fn required_blocked_by_subdirectory() {
        // Leaf has x in ALL its files, but a sibling subdirectory doesn't.
        // dir.all won't have x → no required, even though the leaf's all has it.
        // Allowed stays as * (leaf uncollapsed) since dir.any also lacks x.
        let result = infer_field_paths(&[
            obs("dir/a.md", &["title", "x"]),
            obs("dir/b.md", &["title", "x"]),
            obs("dir/sub/c.md", &["title"]),
        ]);
        // dir leaf: all={t,x}, any={t,x}. sub leaf: all={t}, any={t}.
        // sub/: all={t}, any={t}. dir/: all={t}, any={t,x}∩{t} = {t}.
        // x NOT in dir.any → no collapse. Leaf keeps *.
        // root: single child → same as dir.
        assert_eq!(result["x"], fp(&["dir/*"], &[]));
    }

    #[test]
    fn parallel_subtrees_different_collapse_depths() {
        // Same field x, but different structure in two branches.
        // Left: x in one sub-branch (a/) but not the other (b/) → stays at left/a/**
        // Right: x in both sub-branches (one fully, one partially) → collapses to right/**
        // Tests that allowed and required can reach different depths independently.
        let result = infer_field_paths(&[
            obs("left/a/f1.md", &["title", "x"]),
            obs("left/a/f2.md", &["title", "x"]),
            obs("left/b/f3.md", &["title"]),
            obs("right/c/f4.md", &["title", "x"]),
            obs("right/c/f5.md", &["title"]),
            obs("right/d/f6.md", &["title", "x"]),
            obs("right/d/f7.md", &["title", "x"]),
        ]);
        // left: a/.all={t,x}, b/.all={t}. left.any = {t,x}∩{t} = {t}. No collapse at left.
        // right: c/.all={t}, c/.any={t,x}. d/.all={t,x}, d/.any={t,x}.
        //   right.any = {t,x}∩{t,x} = {t,x}. right.all = {t}∩{t,x} = {t}.
        //   x in right.any \ right.all → collapse allowed only at right/.
        // required: left/a/** (from a/.all), right/d/** (from d/.all). right/ collapse
        //   for allowed doesn't touch required.
        assert_eq!(
            result["x"],
            fp(&["left/a/**", "right/**"], &["left/a/**", "right/d/**"])
        );
    }

    #[test]
    fn wide_fan_field_in_most_not_all() {
        // 5 sibling directories, x in 4 out of 5.
        // root.any = intersection of all 5 → lacks x because e/ doesn't have it.
        // Each dir with x collapses independently.
        let result = infer_field_paths(&[
            obs("a/f.md", &["title", "x"]),
            obs("b/f.md", &["title", "x"]),
            obs("c/f.md", &["title", "x"]),
            obs("d/f.md", &["title", "x"]),
            obs("e/f.md", &["title"]),
        ]);
        assert_eq!(
            result["x"],
            fp(
                &["a/**", "b/**", "c/**", "d/**"],
                &["a/**", "b/**", "c/**", "d/**"]
            )
        );
    }

    #[test]
    fn asymmetric_tree_depth() {
        // One branch is 1 level deep, the other is 5 levels deep.
        // x appears in both endpoints. Tests collapse across very different depths.
        let result = infer_field_paths(&[
            obs("shallow/f.md", &["title", "x"]),
            obs("deep/a/b/c/d/f.md", &["title", "x"]),
            obs("deep/a/b/c/g.md", &["title"]),
            obs("other/f.md", &["title"]),
        ]);
        // shallow/: all={t,x}. Collapse → Recursive.
        // deep/a/b/c/d/: all={t,x}. Collapse → Recursive.
        // deep/a/b/c/: c-leaf any={t}, d/ any={t,x}. c.any = {t}. No collapse.
        // Chain up: b/, a/, deep/ all have .any={t}. No collapse.
        // root.any = {t,x}∩{t}∩{t} = {t}. No collapse.
        assert_eq!(
            result["x"],
            fp(&["deep/a/b/c/d/**", "shallow/**"], &["deep/a/b/c/d/**", "shallow/**"])
        );
    }

    #[test]
    fn root_leaf_with_mixed_subtree_coverage() {
        // Root file + 4 subdirectories: field x in root + 2 subtrees, not in other 2.
        // Root leaf keeps * (root.any lacks x). Two subtrees get **.
        let result = infer_field_paths(&[
            obs("readme.md", &["title", "featured"]),
            obs("blog/a.md", &["title", "featured"]),
            obs("blog/b.md", &["title"]),
            obs("docs/c.md", &["title"]),
            obs("projects/d.md", &["title", "featured"]),
            obs("projects/e.md", &["title", "featured"]),
        ]);
        // root leaf: any={t,featured}. blog: any={t,featured}. docs: any={t}. projects: any={t,featured}.
        // root.any = {t,featured}∩{t,featured}∩{t}∩{t,featured} = {t}. No collapse at root.
        // blog/: featured in blog.any \ blog.all → collapse allowed only → Recursive.
        // projects/: featured in projects.all → collapse both → Recursive.
        // Root leaf stays Shallow (*).
        assert_eq!(
            result["featured"],
            fp(&["*", "blog/**", "projects/**"], &["projects/**"])
        );
    }

    #[test]
    fn many_fields_different_collapse_depths() {
        // 4 fields, each naturally collapses to a different level.
        // Tests that independent fields don't interfere with each other.
        let result = infer_field_paths(&[
            obs("a/b/c/f1.md", &["title", "everywhere", "mid", "deep", "deepest"]),
            obs("a/b/c/f2.md", &["title", "everywhere", "mid", "deep", "deepest"]),
            obs("a/b/f3.md", &["title", "everywhere", "mid"]),
            obs("a/f4.md", &["title", "everywhere"]),
            obs("x/f5.md", &["title", "everywhere"]),
        ]);
        // everywhere: in every file → root.all → ["**"]/["**"]
        assert_eq!(result["everywhere"], fp(&["**"], &["**"]));
        // mid: in a/b/c/ and a/b/ but not a/ (a-leaf lacks it) and not x/.
        //   a/b/ leaf: all={t,everywhere,mid}. a/b/c/ leaf: all has mid.
        //   b/: children = b-leaf + c/. b.all has mid? b-leaf.all has mid, c/.all has mid.
        //   b.all = {t,everywhere,mid,...} ∩ {t,everywhere,mid,...} → includes mid.
        //   a/: children = a-leaf(no mid) + b/(mid in all). a.any has mid? a-leaf.any ∩ b.any.
        //   a-leaf has {t,everywhere} only. a.any = {t,everywhere}∩{...} = {t,everywhere}. No mid.
        //   So mid stays at b/ level. b/.all has mid → collapse both.
        assert_eq!(result["mid"], fp(&["a/b/**"], &["a/b/**"]));
        // deep: only in a/b/c/. c/ is single child → collapses.
        //   b/: b-leaf lacks deep. b.any = {t,e,mid}∩{t,e,mid,deep,deepest} = {t,e,mid}. No.
        assert_eq!(result["deep"], fp(&["a/b/c/**"], &["a/b/c/**"]));
        // deepest: same as deep — only in a/b/c/, same collapse path.
        assert_eq!(result["deepest"], fp(&["a/b/c/**"], &["a/b/c/**"]));
    }

    #[test]
    fn diamond_like_convergence() {
        // Two branches share a field, converge at a common ancestor.
        // shared/ has files + two sub-branches, both with x.
        // The leaf and both sub-branches have x → shared.all includes x → **.
        let result = infer_field_paths(&[
            obs("shared/readme.md", &["title", "x"]),
            obs("shared/left/a.md", &["title", "x"]),
            obs("shared/left/b.md", &["title", "x"]),
            obs("shared/right/c.md", &["title", "x"]),
            obs("shared/right/d.md", &["title", "x"]),
            obs("other/e.md", &["title"]),
        ]);
        // shared leaf: all={t,x}. left/: all={t,x}. right/: all={t,x}.
        // shared/: all = {t,x}∩{t,x}∩{t,x} = {t,x}. Collapse both.
        // root.any = {t,x}∩{t} = {t}. No further collapse.
        assert_eq!(result["x"], fp(&["shared/**"], &["shared/**"]));
    }

    #[test]
    fn diamond_broken_by_one_branch() {
        // Same as above but one sub-branch only partially has x.
        // shared.all loses x, but shared.any keeps it.
        let result = infer_field_paths(&[
            obs("shared/readme.md", &["title", "x"]),
            obs("shared/left/a.md", &["title", "x"]),
            obs("shared/left/b.md", &["title", "x"]),
            obs("shared/right/c.md", &["title", "x"]),
            obs("shared/right/d.md", &["title"]),
            obs("other/e.md", &["title"]),
        ]);
        // shared leaf: all={t,x}. left/: all={t,x}. right/: all={t}, any={t,x}.
        // shared/: all = {t,x}∩{t,x}∩{t} = {t}. any = {t,x}∩{t,x}∩{t,x} = {t,x}.
        // x in shared.any \ shared.all → collapse allowed only.
        // required: left/ has x in all → left/**. shared leaf has x but only leaf-level.
        // shared collapse for allowed swallows the leaf's * and left's ** and right's **.
        assert_eq!(result["x"], fp(&["shared/**"], &["shared/left/**"]));
    }

    #[test]
    fn deeply_nested_single_child_chain() {
        // a/b/c/d/e/f/g/h.md — 8 levels deep, single file.
        // Everything collapses all the way to root.
        let result = infer_field_paths(&[obs("a/b/c/d/e/f/g/h.md", &["title", "x"])]);
        assert_eq!(result["x"], fp(&["**"], &["**"]));
    }

    #[test]
    fn three_fields_three_behaviors_same_subtree() {
        // In left/: alpha in all, beta in any\all, gamma absent.
        // In right/: all three present in all files.
        // Tests that the three collapse categories work independently per subtree.
        let result = infer_field_paths(&[
            obs("left/a.md", &["title", "alpha", "beta"]),
            obs("left/b.md", &["title", "alpha"]),
            obs("right/c.md", &["title", "alpha", "beta", "gamma"]),
            obs("right/d.md", &["title", "alpha", "beta", "gamma"]),
        ]);
        // left: all={t,alpha}, any={t,alpha,beta}. right: all={t,alpha,beta,gamma}, any=same.
        // root: all = {t,alpha}∩{t,alpha,beta,gamma} = {t,alpha}.
        //       any = {t,alpha,beta}∩{t,alpha,beta,gamma} = {t,alpha,beta}.
        // alpha: in root.all → collapse both → **/**
        assert_eq!(result["alpha"], fp(&["**"], &["**"]));
        // beta: in root.any \ root.all → collapse allowed only.
        //   required: right/ has beta in all → right/**. left/ doesn't.
        assert_eq!(result["beta"], fp(&["**"], &["right/**"]));
        // gamma: not in root.any (left doesn't have it) → stays at right.
        //   right.all has gamma → collapse both at right.
        assert_eq!(result["gamma"], fp(&["right/**"], &["right/**"]));
    }

    #[test]
    fn multiple_files_same_dir_with_varied_field_subsets() {
        // 6 files in one directory with overlapping but varied field sets.
        // Tests the all/any computation with many files.
        let result = infer_field_paths(&[
            obs("vault/f1.md", &["title", "tags", "date"]),
            obs("vault/f2.md", &["title", "tags"]),
            obs("vault/f3.md", &["title", "date", "draft"]),
            obs("vault/f4.md", &["title", "tags", "draft"]),
            obs("vault/f5.md", &["title"]),
            obs("vault/f6.md", &["title", "tags", "date", "draft"]),
        ]);
        // vault leaf: all = {title} (f5 has only title).
        //   any = {title, tags, date, draft} (union of all).
        // vault/ → root collapses.
        // title: in all → **/**
        assert_eq!(result["title"], fp(&["**"], &["**"]));
        // tags (4/6), date (3/6), draft (3/6): in any \ all → **/[]
        assert_eq!(result["tags"], fp(&["**"], &[]));
        assert_eq!(result["date"], fp(&["**"], &[]));
        assert_eq!(result["draft"], fp(&["**"], &[]));
    }
}
