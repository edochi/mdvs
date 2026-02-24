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
}
