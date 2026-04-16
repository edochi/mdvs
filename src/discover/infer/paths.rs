//! Structure inference — directory tree construction and glob pattern collapsing.

use crate::discover::scan::ScannedFiles;
use indextree::{Arena, NodeEdge, NodeId};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Inferred glob patterns for a field's allowed and required paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldPaths {
    /// Glob patterns where this field may appear.
    pub allowed: Vec<String>,
    /// Glob patterns where this field must appear (present in every file).
    pub required: Vec<String>,
}

/// Tree of directories used to collapse per-directory field sets into glob patterns.
pub struct DirectoryTree {
    arena: Arena<NodeData>,
    root: NodeId,
}

struct NodeData {
    path: PathBuf,
    all: HashSet<String>,
    any: HashSet<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobDepth {
    Shallow,
    Recursive,
}

impl From<&ScannedFiles> for DirectoryTree {
    fn from(scanned: &ScannedFiles) -> Self {
        let mut arena = Arena::new();
        let root = arena.new_node(NodeData {
            path: PathBuf::new(),
            all: HashSet::new(),
            any: HashSet::new(),
        });

        let mut dir_map: HashMap<PathBuf, NodeId> = HashMap::new();
        dir_map.insert(PathBuf::new(), root);

        let mut leaf_map: HashMap<PathBuf, NodeId> = HashMap::new();

        for file in &scanned.files {
            let fields: HashSet<String> = match &file.data {
                Some(Value::Object(map)) => map.keys().cloned().collect(),
                _ => HashSet::new(),
            };

            let parent_dir = file.path.parent().unwrap_or(Path::new("")).to_path_buf();
            let dir_node_id = ensure_dir(&mut arena, &mut dir_map, root, &parent_dir);

            let leaf_id = *leaf_map.entry(parent_dir.clone()).or_insert_with(|| {
                let leaf = arena.new_node(NodeData {
                    path: parent_dir,
                    all: fields.clone(),
                    any: fields.clone(),
                });
                dir_node_id.append(leaf, &mut arena);
                leaf
            });

            let node = arena[leaf_id].get_mut();
            node.all = node.all.intersection(&fields).cloned().collect();
            node.any = node.any.union(&fields).cloned().collect();
        }

        let mut tree = DirectoryTree { arena, root };
        tree.merge();
        tree
    }
}

fn ensure_dir(
    arena: &mut Arena<NodeData>,
    dir_map: &mut HashMap<PathBuf, NodeId>,
    root: NodeId,
    dir_path: &Path,
) -> NodeId {
    if let Some(&id) = dir_map.get(dir_path) {
        return id;
    }

    let mut to_create = Vec::new();
    let mut current = dir_path.to_path_buf();
    while !dir_map.contains_key(&current) {
        to_create.push(current.clone());
        current = current.parent().unwrap_or(Path::new("")).to_path_buf();
    }

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

fn intersect_all(sets: &[HashSet<String>]) -> HashSet<String> {
    let Some(first) = sets.first() else {
        return HashSet::new();
    };
    let mut result = first.clone();
    for set in &sets[1..] {
        result = result.intersection(set).cloned().collect();
    }
    result
}

struct GlobMap {
    entries: HashMap<PathBuf, GlobDepth>,
}

impl GlobMap {
    fn new() -> Self {
        GlobMap {
            entries: HashMap::new(),
        }
    }

    fn insert_shallow(&mut self, path: PathBuf) {
        self.entries.insert(path, GlobDepth::Shallow);
    }

    fn collapse(&mut self, ancestor_path: &Path) {
        self.entries.retain(|p, _| !p.starts_with(ancestor_path));
        self.entries
            .insert(ancestor_path.to_path_buf(), GlobDepth::Recursive);
    }

    fn to_globs(&self) -> Vec<String> {
        let mut globs: Vec<String> = self
            .entries
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
}

impl DirectoryTree {
    /// Walk the tree bottom-up, collapsing leaf directories into glob patterns.
    #[instrument(name = "infer_paths", skip_all, level = "debug")]
    pub fn infer_paths(&self) -> BTreeMap<String, FieldPaths> {
        let mut allowed: HashMap<String, GlobMap> = HashMap::new();
        let mut required: HashMap<String, GlobMap> = HashMap::new();

        for node_id in self.root.descendants(&self.arena) {
            if self.arena[node_id].first_child().is_some() {
                continue;
            }
            let node = self.arena[node_id].get();
            for field in &node.any {
                allowed
                    .entry(field.clone())
                    .or_insert_with(GlobMap::new)
                    .insert_shallow(node.path.clone());
            }
        }

        let post_order: Vec<NodeId> = self
            .root
            .traverse(&self.arena)
            .filter_map(|edge| match edge {
                NodeEdge::End(id) => Some(id),
                _ => None,
            })
            .collect();

        for node_id in post_order {
            if self.arena[node_id].first_child().is_none() {
                continue;
            }

            let node = self.arena[node_id].get();
            let node_path = &node.path;

            for field in &node.all {
                allowed
                    .entry(field.clone())
                    .or_insert_with(GlobMap::new)
                    .collapse(node_path);
                required
                    .entry(field.clone())
                    .or_insert_with(GlobMap::new)
                    .collapse(node_path);
            }

            for field in node.any.difference(&node.all) {
                allowed
                    .entry(field.clone())
                    .or_insert_with(GlobMap::new)
                    .collapse(node_path);
            }
        }

        let all_fields: HashSet<&String> = allowed.keys().chain(required.keys()).collect();
        let mut result = BTreeMap::new();
        for field in all_fields {
            result.insert(
                field.clone(),
                FieldPaths {
                    allowed: allowed.get(field).map(|g| g.to_globs()).unwrap_or_default(),
                    required: required
                        .get(field)
                        .map(|g| g.to_globs())
                        .unwrap_or_default(),
                },
            );
        }
        result
    }

    fn merge(&mut self) {
        let post_order: Vec<NodeId> = self
            .root
            .traverse(&self.arena)
            .filter_map(|edge| match edge {
                NodeEdge::End(id) => Some(id),
                _ => None,
            })
            .collect();

        for node_id in post_order {
            if self.arena[node_id].first_child().is_none() {
                continue;
            }

            let mut child_all: Vec<HashSet<String>> = Vec::new();
            let mut child_any: Vec<HashSet<String>> = Vec::new();
            let mut child_id = self.arena[node_id].first_child();
            while let Some(cid) = child_id {
                let child = self.arena[cid].get();
                child_all.push(child.all.clone());
                child_any.push(child.any.clone());
                child_id = self.arena[cid].next_sibling();
            }

            let merged_all = intersect_all(&child_all);
            let merged_any = intersect_all(&child_any);

            let node = self.arena[node_id].get_mut();
            node.all = merged_all;
            node.any = merged_any;
        }
    }
}
