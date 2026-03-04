use crate::discover::field_type::{widen, FieldType};
use crate::discover::scan::ScannedFiles;
use indextree::{Arena, NodeEdge, NodeId};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use tracing::{info, instrument};

// ============================================================================
// Type inference — flat pass
// ============================================================================

/// Widened type and file list for a single frontmatter field.
#[derive(Debug)]
pub struct FieldTypeInfo {
    /// Widened type across all files where this field appears.
    pub field_type: FieldType,
    /// Paths of files containing this field.
    pub files: Vec<PathBuf>,
}

/// Infer field types by scanning all files and widening across occurrences.
#[instrument(name = "infer_types", skip_all, level = "debug")]
pub fn infer_field_types(scanned: &ScannedFiles) -> BTreeMap<String, FieldTypeInfo> {
    let mut types: BTreeMap<String, FieldType> = BTreeMap::new();
    let mut files: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    for file in &scanned.files {
        if let Some(Value::Object(map)) = &file.data {
            for (key, val) in map {
                // Always track file presence
                files
                    .entry(key.clone())
                    .or_default()
                    .push(file.path.clone());

                // Skip null — transparent in type inference
                if val.is_null() {
                    continue;
                }

                let ft = FieldType::from(val);
                types
                    .entry(key.clone())
                    .and_modify(|existing| *existing = widen(existing.clone(), ft.clone()))
                    .or_insert(ft);
            }
        }
    }

    // Fields present only as null default to String
    for key in files.keys() {
        types.entry(key.clone()).or_insert(FieldType::String);
    }

    types
        .into_iter()
        .map(|(name, field_type)| {
            let info = FieldTypeInfo {
                field_type,
                files: files.remove(&name).unwrap_or_default(),
            };
            (name, info)
        })
        .collect()
}

// ============================================================================
// Structure inference — DirectoryTree
// ============================================================================

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
        self.entries
            .retain(|p, _| !p.starts_with(ancestor_path));
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
                    allowed: allowed
                        .get(field)
                        .map(|g| g.to_globs())
                        .unwrap_or_default(),
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

// ============================================================================
// InferredField / InferredSchema
// ============================================================================

/// A single field inferred from scanning: type, file list, and glob patterns.
#[derive(Debug)]
pub struct InferredField {
    /// Field name (frontmatter key).
    pub name: String,
    /// Widened type across all occurrences.
    pub field_type: FieldType,
    /// Paths of files containing this field.
    pub files: Vec<PathBuf>,
    /// Glob patterns where this field may appear.
    pub allowed: Vec<String>,
    /// Glob patterns where this field is present in every file.
    pub required: Vec<String>,
}

/// Complete inferred schema: all fields with types and path constraints.
#[derive(Debug)]
pub struct InferredSchema {
    /// Fields sorted by name.
    pub fields: Vec<InferredField>,
}

impl InferredSchema {
    /// Run full inference: types + directory tree → fields with glob patterns.
    #[instrument(name = "infer", skip_all)]
    pub fn infer(scanned: &ScannedFiles) -> Self {
        let mut type_info = infer_field_types(scanned);
        let tree = DirectoryTree::from(scanned);
        let path_info = tree.infer_paths();

        let mut fields: Vec<InferredField> = type_info
            .keys()
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .map(|name| {
                let ti = type_info.remove(&name).unwrap();
                let pi = path_info.get(&name);
                InferredField {
                    field_type: ti.field_type,
                    files: ti.files,
                    allowed: pi.map(|p| p.allowed.clone()).unwrap_or_default(),
                    required: pi.map(|p| p.required.clone()).unwrap_or_default(),
                    name,
                }
            })
            .collect();

        fields.sort_by(|a, b| a.name.cmp(&b.name));

        info!(fields = fields.len(), "inference complete");

        InferredSchema { fields }
    }

    /// Look up a field by name.
    pub fn field(&self, name: &str) -> Option<&InferredField> {
        self.fields.iter().find(|f| f.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::scan::ScannedFile;

    fn sf(path: &str, data: Option<Value>, content: &str) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(path),
            data,
            content: content.to_string(),
        }
    }

    #[test]
    fn empty_input() {
        let scanned = ScannedFiles { files: vec![] };
        let schema = InferredSchema::infer(&scanned);
        assert!(schema.fields.is_empty());
    }

    #[test]
    fn files_without_frontmatter() {
        let scanned = ScannedFiles {
            files: vec![
                sf("notes/bare.md", None, "No frontmatter here."),
                sf("notes/also_bare.md", None, "Also no frontmatter."),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        assert!(schema.fields.is_empty());
    }

    #[test]
    fn single_file_all_fields() {
        let scanned = ScannedFiles {
            files: vec![sf(
                "blog/post.md",
                Some(serde_json::json!({"title": "Hello", "draft": true, "count": 42})),
                "Body.",
            )],
        };
        let schema = InferredSchema::infer(&scanned);
        assert_eq!(schema.fields.len(), 3);

        let title = schema.field("title").unwrap();
        assert_eq!(title.field_type, FieldType::String);
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["**"]);
        assert_eq!(title.files, vec![PathBuf::from("blog/post.md")]);

        let draft = schema.field("draft").unwrap();
        assert_eq!(draft.field_type, FieldType::Boolean);

        let count = schema.field("count").unwrap();
        assert_eq!(count.field_type, FieldType::Integer);
    }

    #[test]
    fn type_widening_int_float() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"val": 42})), ""),
                sf("b.md", Some(serde_json::json!({"val": 2.72})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let val = schema.field("val").unwrap();
        assert_eq!(val.field_type, FieldType::Float);
        assert_eq!(val.files.len(), 2);
    }

    #[test]
    fn incompatible_types_widen_to_string() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"val": true})), ""),
                sf("b.md", Some(serde_json::json!({"val": 42})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let val = schema.field("val").unwrap();
        assert_eq!(val.field_type, FieldType::String);
    }

    #[test]
    fn partial_field_coverage() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/a.md", Some(serde_json::json!({"title": "A", "tags": ["rust"]})), ""),
                sf("blog/b.md", Some(serde_json::json!({"title": "B"})), ""),
                sf("notes/c.md", Some(serde_json::json!({"title": "C", "tags": ["idea"]})), ""),
                sf("notes/d.md", Some(serde_json::json!({"title": "D", "tags": ["todo"]})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["**"]);

        let tags = schema.field("tags").unwrap();
        assert_eq!(tags.field_type, FieldType::Array(Box::new(FieldType::String)));
        assert_eq!(tags.allowed, vec!["**"]);
        assert_eq!(tags.required, vec!["notes/**"]);
    }

    #[test]
    fn disjoint_fields_in_different_dirs() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/a.md", Some(serde_json::json!({"title": "A", "tags": ["rust"]})), ""),
                sf("blog/b.md", Some(serde_json::json!({"title": "B", "tags": ["go"]})), ""),
                sf("papers/x.md", Some(serde_json::json!({"title": "X", "doi": "10.1234"})), ""),
                sf("papers/y.md", Some(serde_json::json!({"title": "Y", "doi": "10.5678"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        assert_eq!(schema.field("title").unwrap().allowed, vec!["**"]);
        assert_eq!(schema.field("title").unwrap().required, vec!["**"]);
        assert_eq!(schema.field("tags").unwrap().allowed, vec!["blog/**"]);
        assert_eq!(schema.field("tags").unwrap().required, vec!["blog/**"]);
        assert_eq!(schema.field("doi").unwrap().allowed, vec!["papers/**"]);
        assert_eq!(schema.field("doi").unwrap().required, vec!["papers/**"]);
    }

    #[test]
    fn object_widening_across_files() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"meta": {"author": "Alice", "version": 1}})), ""),
                sf("b.md", Some(serde_json::json!({"meta": {"author": "Bob", "license": "MIT"}})), ""),
                sf("c.md", Some(serde_json::json!({"meta": {"author": "Charlie", "version": 2.0}})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let meta = schema.field("meta").unwrap();
        assert_eq!(
            meta.field_type,
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("license".into(), FieldType::String),
                ("version".into(), FieldType::Float),
            ]))
        );
        assert_eq!(meta.files.len(), 3);
    }

    #[test]
    fn mixed_files_with_and_without_frontmatter() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/post.md", Some(serde_json::json!({"title": "Hello", "draft": true})), "Post body."),
                sf("blog/bare.md", None, "No frontmatter."),
                sf("notes/idea.md", Some(serde_json::json!({"title": "Idea"})), "Idea body."),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.files.len(), 2);
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["notes/**"]);

        let draft = schema.field("draft").unwrap();
        assert_eq!(draft.files.len(), 1);
        assert_eq!(draft.allowed, vec!["blog/**"]);
        assert!(draft.required.is_empty());
    }

    #[test]
    fn deep_nesting_with_partial_collapse() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/posts/a.md", Some(serde_json::json!({"title": "A", "tags": ["rust"]})), ""),
                sf("blog/posts/b.md", Some(serde_json::json!({"title": "B", "tags": ["go"]})), ""),
                sf("blog/drafts/c.md", Some(serde_json::json!({"title": "C", "draft": true})), ""),
                sf("papers/x.md", Some(serde_json::json!({"title": "X", "doi": "10.1234"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        assert_eq!(schema.field("title").unwrap().allowed, vec!["**"]);
        assert_eq!(schema.field("title").unwrap().required, vec!["**"]);
        assert_eq!(schema.field("tags").unwrap().allowed, vec!["blog/posts/**"]);
        assert_eq!(schema.field("tags").unwrap().required, vec!["blog/posts/**"]);
        assert_eq!(schema.field("draft").unwrap().allowed, vec!["blog/drafts/**"]);
        assert_eq!(schema.field("draft").unwrap().required, vec!["blog/drafts/**"]);
        assert_eq!(schema.field("doi").unwrap().allowed, vec!["papers/**"]);
        assert_eq!(schema.field("doi").unwrap().required, vec!["papers/**"]);
    }

    #[test]
    fn array_type_inference() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"items": [1, 2, 3]})), ""),
                sf("b.md", Some(serde_json::json!({"items": [4.5, 5.5]})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let items = schema.field("items").unwrap();
        assert_eq!(items.field_type, FieldType::Array(Box::new(FieldType::Float)));
    }

    #[test]
    fn worked_example_from_spec() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/post1.md", Some(serde_json::json!({"title": "P1", "tags": ["a"]})), ""),
                sf("blog/post2.md", Some(serde_json::json!({"title": "P2"})), ""),
                sf("blog/drafts/d1.md", Some(serde_json::json!({"title": "D1", "tags": ["b"]})), ""),
                sf("blog/drafts/d2.md", Some(serde_json::json!({"title": "D2", "tags": ["c"]})), ""),
                sf("notes/idea1.md", Some(serde_json::json!({"title": "I1", "tags": ["d"]})), ""),
                sf("notes/idea2.md", Some(serde_json::json!({"title": "I2", "tags": ["e"]})), ""),
                sf("papers/paper1.md", Some(serde_json::json!({"title": "P1"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["**"]);

        let tags = schema.field("tags").unwrap();
        assert_eq!(tags.allowed, vec!["blog/**", "notes/**"]);
        assert_eq!(tags.required, vec!["blog/drafts/**", "notes/**"]);
    }

    #[test]
    fn fields_sorted_by_name() {
        let scanned = ScannedFiles {
            files: vec![sf(
                "a.md",
                Some(serde_json::json!({"zebra": 1, "alpha": 2, "middle": 3})),
                "",
            )],
        };
        let schema = InferredSchema::infer(&scanned);
        let names: Vec<&str> = schema.fields.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "middle", "zebra"]);
    }

    #[test]
    fn files_list_tracks_field_presence() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"title": "A", "extra": true})), ""),
                sf("b.md", Some(serde_json::json!({"title": "B"})), ""),
                sf("c.md", Some(serde_json::json!({"title": "C", "extra": false})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        assert_eq!(schema.field("title").unwrap().files.len(), 3);

        let extra = schema.field("extra").unwrap();
        assert_eq!(extra.files.len(), 2);
        assert!(extra.files.contains(&PathBuf::from("a.md")));
        assert!(extra.files.contains(&PathBuf::from("c.md")));
    }

    #[test]
    fn complex_real_world_scenario() {
        let scanned = ScannedFiles {
            files: vec![
                sf("blog/2024/jan.md", Some(serde_json::json!({"title": "Jan", "tags": ["update"], "draft": false})), ""),
                sf("blog/2024/feb.md", Some(serde_json::json!({"title": "Feb", "tags": ["release"]})), ""),
                sf("blog/2025/mar.md", Some(serde_json::json!({"title": "Mar", "tags": ["news"], "draft": true})), ""),
                sf("papers/p1.md", Some(serde_json::json!({"title": "Paper1", "doi": "10.1000/1"})), ""),
                sf("papers/p2.md", Some(serde_json::json!({"title": "Paper2", "doi": "10.1000/2"})), ""),
                sf("notes/idea.md", Some(serde_json::json!({"title": "Idea"})), ""),
                sf("readme.md", None, "# README"),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.field_type, FieldType::String);
        assert_eq!(title.files.len(), 6);
        assert_eq!(title.allowed, vec!["blog/**", "notes/**", "papers/**"]);
        assert_eq!(title.required, vec!["blog/**", "notes/**", "papers/**"]);

        let tags = schema.field("tags").unwrap();
        assert_eq!(tags.field_type, FieldType::Array(Box::new(FieldType::String)));
        assert_eq!(tags.files.len(), 3);
        assert_eq!(tags.allowed, vec!["blog/**"]);
        assert_eq!(tags.required, vec!["blog/**"]);

        let draft = schema.field("draft").unwrap();
        assert_eq!(draft.field_type, FieldType::Boolean);
        assert_eq!(draft.files.len(), 2);
        assert_eq!(draft.allowed, vec!["blog/**"]);
        assert_eq!(draft.required, vec!["blog/2025/**"]);

        let doi = schema.field("doi").unwrap();
        assert_eq!(doi.field_type, FieldType::String);
        assert_eq!(doi.files.len(), 2);
        assert_eq!(doi.allowed, vec!["papers/**"]);
        assert_eq!(doi.required, vec!["papers/**"]);
    }

    #[test]
    fn three_way_widening() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"val": 42})), ""),
                sf("b.md", Some(serde_json::json!({"val": 2.72})), ""),
                sf("c.md", Some(serde_json::json!({"val": "hello"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        assert_eq!(schema.field("val").unwrap().field_type, FieldType::String);
    }

    #[test]
    fn null_values_become_string() {
        let scanned = ScannedFiles {
            files: vec![sf("a.md", Some(serde_json::json!({"val": null})), "")],
        };
        let schema = InferredSchema::infer(&scanned);
        assert_eq!(schema.field("val").unwrap().field_type, FieldType::String);
    }

    #[test]
    fn null_transparent_in_widening() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"projects": null})), ""),
                sf(
                    "b.md",
                    Some(serde_json::json!({"projects": ["Foo"]})),
                    "",
                ),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let projects = schema.field("projects").unwrap();
        assert_eq!(
            projects.field_type,
            FieldType::Array(Box::new(FieldType::String))
        );
        assert_eq!(projects.files.len(), 2);
    }

    #[test]
    fn null_plus_int_infers_int() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(serde_json::json!({"count": null})), ""),
                sf("b.md", Some(serde_json::json!({"count": 42})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        assert_eq!(
            schema.field("count").unwrap().field_type,
            FieldType::Integer
        );
    }

    #[test]
    fn root_files_shallow_glob() {
        let scanned = ScannedFiles {
            files: vec![
                sf("readme.md", Some(serde_json::json!({"title": "Root", "featured": true})), ""),
                sf("blog/a.md", Some(serde_json::json!({"title": "Blog"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["**"]);

        let featured = schema.field("featured").unwrap();
        assert_eq!(featured.allowed, vec!["*"]);
        assert!(featured.required.is_empty());
    }
}
