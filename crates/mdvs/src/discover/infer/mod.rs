//! Schema inference: types, directory structure, and categorical constraints.
//!
//! Submodules:
//! - [`types`] — type widening and distinct value collection
//! - [`paths`] — directory tree construction and glob pattern collapsing
//! - [`constraints`] — categorical heuristic detection

pub mod constraints;
mod paths;
mod types;

pub use constraints::infer_constraints;
pub use paths::{DirectoryTree, FieldPaths};
pub use types::{FieldTypeInfo, infer_field_types};

use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::output::DiscoveredField;
use crate::schema::shared::FieldTypeSerde;
use serde_json::Value;
use std::path::PathBuf;
use tracing::{info, instrument};

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
    /// Whether any file had a null value for this field.
    pub nullable: bool,
    /// Distinct non-null values, post-processed for the widened type:
    /// element-level for Array fields, serialized for String fields.
    pub distinct_values: Vec<Value>,
    /// Total non-null value count (element-level for Array fields,
    /// value-level for all others).
    pub occurrence_count: usize,
}

impl InferredField {
    /// Convert to a [`DiscoveredField`] for command output.
    pub fn to_discovered(&self, total_files: usize, verbose: bool) -> DiscoveredField {
        DiscoveredField {
            name: self.name.clone(),
            field_type: FieldTypeSerde::from(&self.field_type).to_string(),
            files_found: self.files.len(),
            total_files,
            allowed: if verbose {
                Some(self.allowed.clone())
            } else {
                None
            },
            required: if verbose {
                Some(self.required.clone())
            } else {
                None
            },
            nullable: self.nullable,
            hints: crate::output::field_hints(&self.name),
        }
    }
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
                    nullable: ti.nullable,
                    distinct_values: ti.distinct_values,
                    occurrence_count: ti.occurrence_count,
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
    use crate::discover::scan::{ScannedFile, ScannedFiles};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn sf(path: &str, data: Option<Value>, content: &str) -> ScannedFile {
        ScannedFile {
            path: PathBuf::from(path),
            data,
            content: content.to_string(),
            body_line_offset: 0,
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
                sf(
                    "blog/a.md",
                    Some(serde_json::json!({"title": "A", "tags": ["rust"]})),
                    "",
                ),
                sf("blog/b.md", Some(serde_json::json!({"title": "B"})), ""),
                sf(
                    "notes/c.md",
                    Some(serde_json::json!({"title": "C", "tags": ["idea"]})),
                    "",
                ),
                sf(
                    "notes/d.md",
                    Some(serde_json::json!({"title": "D", "tags": ["todo"]})),
                    "",
                ),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let title = schema.field("title").unwrap();
        assert_eq!(title.allowed, vec!["**"]);
        assert_eq!(title.required, vec!["**"]);

        let tags = schema.field("tags").unwrap();
        assert_eq!(
            tags.field_type,
            FieldType::Array(Box::new(FieldType::String))
        );
        assert_eq!(tags.allowed, vec!["**"]);
        assert_eq!(tags.required, vec!["notes/**"]);
    }

    #[test]
    fn disjoint_fields_in_different_dirs() {
        let scanned = ScannedFiles {
            files: vec![
                sf(
                    "blog/a.md",
                    Some(serde_json::json!({"title": "A", "tags": ["rust"]})),
                    "",
                ),
                sf(
                    "blog/b.md",
                    Some(serde_json::json!({"title": "B", "tags": ["go"]})),
                    "",
                ),
                sf(
                    "papers/x.md",
                    Some(serde_json::json!({"title": "X", "doi": "10.1234"})),
                    "",
                ),
                sf(
                    "papers/y.md",
                    Some(serde_json::json!({"title": "Y", "doi": "10.5678"})),
                    "",
                ),
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
                sf(
                    "a.md",
                    Some(serde_json::json!({"meta": {"author": "Alice", "version": 1}})),
                    "",
                ),
                sf(
                    "b.md",
                    Some(serde_json::json!({"meta": {"author": "Bob", "license": "MIT"}})),
                    "",
                ),
                sf(
                    "c.md",
                    Some(serde_json::json!({"meta": {"author": "Charlie", "version": 2.0}})),
                    "",
                ),
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
                sf(
                    "blog/post.md",
                    Some(serde_json::json!({"title": "Hello", "draft": true})),
                    "Post body.",
                ),
                sf("blog/bare.md", None, "No frontmatter."),
                sf(
                    "notes/idea.md",
                    Some(serde_json::json!({"title": "Idea"})),
                    "Idea body.",
                ),
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
                sf(
                    "blog/posts/a.md",
                    Some(serde_json::json!({"title": "A", "tags": ["rust"]})),
                    "",
                ),
                sf(
                    "blog/posts/b.md",
                    Some(serde_json::json!({"title": "B", "tags": ["go"]})),
                    "",
                ),
                sf(
                    "blog/drafts/c.md",
                    Some(serde_json::json!({"title": "C", "draft": true})),
                    "",
                ),
                sf(
                    "papers/x.md",
                    Some(serde_json::json!({"title": "X", "doi": "10.1234"})),
                    "",
                ),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        assert_eq!(schema.field("title").unwrap().allowed, vec!["**"]);
        assert_eq!(schema.field("title").unwrap().required, vec!["**"]);
        assert_eq!(schema.field("tags").unwrap().allowed, vec!["blog/posts/**"]);
        assert_eq!(
            schema.field("tags").unwrap().required,
            vec!["blog/posts/**"]
        );
        assert_eq!(
            schema.field("draft").unwrap().allowed,
            vec!["blog/drafts/**"]
        );
        assert_eq!(
            schema.field("draft").unwrap().required,
            vec!["blog/drafts/**"]
        );
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
        assert_eq!(
            items.field_type,
            FieldType::Array(Box::new(FieldType::Float))
        );
    }

    #[test]
    fn worked_example_from_spec() {
        let scanned = ScannedFiles {
            files: vec![
                sf(
                    "blog/post1.md",
                    Some(serde_json::json!({"title": "P1", "tags": ["a"]})),
                    "",
                ),
                sf(
                    "blog/post2.md",
                    Some(serde_json::json!({"title": "P2"})),
                    "",
                ),
                sf(
                    "blog/drafts/d1.md",
                    Some(serde_json::json!({"title": "D1", "tags": ["b"]})),
                    "",
                ),
                sf(
                    "blog/drafts/d2.md",
                    Some(serde_json::json!({"title": "D2", "tags": ["c"]})),
                    "",
                ),
                sf(
                    "notes/idea1.md",
                    Some(serde_json::json!({"title": "I1", "tags": ["d"]})),
                    "",
                ),
                sf(
                    "notes/idea2.md",
                    Some(serde_json::json!({"title": "I2", "tags": ["e"]})),
                    "",
                ),
                sf(
                    "papers/paper1.md",
                    Some(serde_json::json!({"title": "P1"})),
                    "",
                ),
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
                sf(
                    "a.md",
                    Some(serde_json::json!({"title": "A", "extra": true})),
                    "",
                ),
                sf("b.md", Some(serde_json::json!({"title": "B"})), ""),
                sf(
                    "c.md",
                    Some(serde_json::json!({"title": "C", "extra": false})),
                    "",
                ),
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
                sf(
                    "blog/2024/jan.md",
                    Some(serde_json::json!({"title": "Jan", "tags": ["update"], "draft": false})),
                    "",
                ),
                sf(
                    "blog/2024/feb.md",
                    Some(serde_json::json!({"title": "Feb", "tags": ["release"]})),
                    "",
                ),
                sf(
                    "blog/2025/mar.md",
                    Some(serde_json::json!({"title": "Mar", "tags": ["news"], "draft": true})),
                    "",
                ),
                sf(
                    "papers/p1.md",
                    Some(serde_json::json!({"title": "Paper1", "doi": "10.1000/1"})),
                    "",
                ),
                sf(
                    "papers/p2.md",
                    Some(serde_json::json!({"title": "Paper2", "doi": "10.1000/2"})),
                    "",
                ),
                sf(
                    "notes/idea.md",
                    Some(serde_json::json!({"title": "Idea"})),
                    "",
                ),
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
        assert_eq!(
            tags.field_type,
            FieldType::Array(Box::new(FieldType::String))
        );
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
                sf("b.md", Some(serde_json::json!({"projects": ["Foo"]})), ""),
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
                sf(
                    "readme.md",
                    Some(serde_json::json!({"title": "Root", "featured": true})),
                    "",
                ),
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

    // -----------------------------------------------------------------------
    // Categorical inference — pipeline integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn null_values_excluded_from_distinct() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"status": "draft"})), ""),
                sf("b.md", Some(json!({"status": "draft"})), ""),
                sf("c.md", Some(json!({"status": null})), ""),
                sf("d.md", Some(json!({"status": "published"})), ""),
                sf("e.md", Some(json!({"status": "published"})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let status = schema.field("status").unwrap();
        assert_eq!(status.distinct_values.len(), 2);
        assert_eq!(status.occurrence_count, 4);
    }

    #[test]
    fn full_pipeline_categorical_inference() {
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"status": "draft", "title": "A"})), ""),
                sf("b.md", Some(json!({"status": "draft", "title": "B"})), ""),
                sf(
                    "c.md",
                    Some(json!({"status": "published", "title": "C"})),
                    "",
                ),
                sf(
                    "d.md",
                    Some(json!({"status": "published", "title": "D"})),
                    "",
                ),
                sf(
                    "e.md",
                    Some(json!({"status": "archived", "title": "E"})),
                    "",
                ),
                sf(
                    "f.md",
                    Some(json!({"status": "archived", "title": "F"})),
                    "",
                ),
            ],
        };
        let schema = InferredSchema::infer(&scanned);

        let status = schema.field("status").unwrap();
        assert_eq!(status.distinct_values.len(), 3);
        assert_eq!(status.occurrence_count, 6);
        let c = infer_constraints(status, 10, 2).unwrap();
        assert_eq!(c.categories.unwrap().len(), 3);

        let title = schema.field("title").unwrap();
        assert_eq!(title.distinct_values.len(), 6);
        assert!(infer_constraints(title, 10, 2).is_none());
    }

    #[test]
    fn mixed_string_array_categorical_inference() {
        // funding: "internal" in some files, funding: ["internal"] in others.
        // Widens to String. Categories should include both forms.
        let scanned = ScannedFiles {
            files: vec![
                sf("a.md", Some(json!({"funding": "internal"})), ""),
                sf("b.md", Some(json!({"funding": "internal"})), ""),
                sf("c.md", Some(json!({"funding": "internal"})), ""),
                sf("d.md", Some(json!({"funding": ["internal"]})), ""),
                sf("e.md", Some(json!({"funding": ["internal"]})), ""),
                sf("f.md", Some(json!({"funding": ["internal"]})), ""),
            ],
        };
        let schema = InferredSchema::infer(&scanned);
        let funding = schema.field("funding").unwrap();
        assert_eq!(funding.field_type, FieldType::String);
        assert_eq!(funding.distinct_values.len(), 2);
        assert_eq!(funding.occurrence_count, 6);

        // 6 occurrences / 2 distinct = 3 >= min_repetition → categories inferred
        let c = infer_constraints(funding, 10, 3).unwrap();
        let cats = c.categories.unwrap();
        assert_eq!(cats.len(), 2);
        assert!(cats.contains(&toml::Value::String("internal".into())));
        assert!(cats.contains(&toml::Value::String(r#"["internal"]"#.into())));
    }
}
