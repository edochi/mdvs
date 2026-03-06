use crate::discover::infer::InferredSchema;
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use tracing::instrument;

/// Controls behavior after `update` completes.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct UpdateConfig {
    /// Whether to automatically trigger a build after updating the schema.
    pub auto_build: bool,
}

/// Default settings for the `search` command.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct SearchConfig {
    /// Maximum number of results returned when `--limit` is not specified.
    pub default_limit: usize,
}

/// Storage-layer settings controlling how parquet columns are named.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct StorageConfig {
    /// Prefix applied to internal parquet column names to avoid collisions
    /// with frontmatter field names. Defaults to `"_"`.
    #[serde(default = "default_internal_prefix")]
    pub internal_prefix: String,
}

fn default_internal_prefix() -> String {
    "_".into()
}

/// A single field definition in `[[fields.field]]`, specifying its type
/// and the glob patterns that constrain where it may or must appear.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TomlField {
    /// Frontmatter key this definition applies to.
    pub name: String,
    /// Expected type (scalar, array, or object).
    #[serde(rename = "type")]
    pub field_type: FieldTypeSerde,
    /// Glob patterns for files where this field is allowed.
    pub allowed: Vec<String>,
    /// Glob patterns for files where this field is required.
    pub required: Vec<String>,
}

/// The `[fields]` section: ignore list and per-field definitions.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct FieldsConfig {
    /// Field names to ignore during validation (known but unconstrained).
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Constrained field definitions (`[[fields.field]]` entries).
    #[serde(default, rename = "field")]
    pub field: Vec<TomlField>,
}

/// Top-level representation of `mdvs.toml`, the single source of truth for
/// schema validation and build configuration. Validation sections (`scan`,
/// `update`, `fields`) are always present; build sections (`embedding_model`,
/// `chunking`, `search`) are optional and added by `init --auto-build` or `build`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MdvsToml {
    /// File discovery settings (glob pattern, bare-file handling).
    pub scan: ScanConfig,
    /// Post-update workflow settings.
    pub update: UpdateConfig,
    /// Embedding model identity. Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<EmbeddingModelConfig>,
    /// Chunk-size settings for semantic splitting. Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunking: Option<ChunkingConfig>,
    /// Search defaults. Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchConfig>,
    /// Storage-layer settings (column prefix). Hidden by default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage: Option<StorageConfig>,
    /// Field definitions and ignore list.
    pub fields: FieldsConfig,
}

impl MdvsToml {
    /// Return the internal column prefix, defaulting to `"_"` when no `[storage]` section is set.
    pub fn internal_prefix(&self) -> &str {
        self.storage.as_ref().map_or("_", |s| &s.internal_prefix)
    }

    /// Build an `MdvsToml` from an inferred schema and the provided scan/model/chunking settings.
    /// When `auto_build` is false, the build sections are omitted.
    pub fn from_inferred(
        schema: &InferredSchema,
        scan: ScanConfig,
        model_name: &str,
        model_revision: Option<&str>,
        max_chunk_size: usize,
        auto_build: bool,
    ) -> Self {
        let (embedding_model, chunking, search) = if auto_build {
            (
                Some(EmbeddingModelConfig {
                    provider: "model2vec".to_string(),
                    name: model_name.to_string(),
                    revision: model_revision.map(|s| s.to_string()),
                }),
                Some(ChunkingConfig { max_chunk_size }),
                Some(SearchConfig { default_limit: 10 }),
            )
        } else {
            (None, None, None)
        };

        MdvsToml {
            scan,
            update: UpdateConfig { auto_build },
            fields: FieldsConfig {
                ignore: vec![],
                field: schema
                    .fields
                    .iter()
                    .map(|f| TomlField {
                        name: f.name.clone(),
                        field_type: FieldTypeSerde::from(&f.field_type),
                        allowed: f.allowed.clone(),
                        required: f.required.clone(),
                    })
                    .collect(),
            },
            storage: None,
            embedding_model,
            chunking,
            search,
        }
    }

    /// Deserialize an `MdvsToml` from a file on disk.
    #[instrument(name = "read_config", skip_all, level = "debug")]
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: MdvsToml = toml::from_str(&content)?;
        Ok(config)
    }

    /// Serialize this config to TOML and write it to disk.
    /// Complex field types are post-processed into inline tables for readability.
    #[instrument(name = "write_config", skip_all, level = "debug")]
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string(self)?;
        let content = inline_field_types(&content)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Post-process serialized TOML to render `type` fields as inline tables.
/// The `toml` crate expands them into `[fields.field.type]` sections;
/// we convert those back to `type = { array = "String" }` form.
fn inline_field_types(toml_str: &str) -> anyhow::Result<String> {
    let mut doc = toml_str.parse::<toml_edit::DocumentMut>()?;

    if let Some(fields) = doc.get_mut("fields") {
        if let Some(field_array) = fields.get_mut("field") {
            if let Some(array_of_tables) = field_array.as_array_of_tables_mut() {
                for table in array_of_tables.iter_mut() {
                    if let Some(type_item) = table.get_mut("type") {
                        if let Some(t) = type_item.as_table_mut() {
                            let inline = t.clone().into_inline_table();
                            *type_item = toml_edit::Item::Value(inline.into());
                        }
                    }
                    // Normalize key formatting (fix missing space before `=`)
                    if let Some(mut key) = table.key_mut("type") {
                        key.fmt();
                    }
                }
            }
        }
    }

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::field_type::FieldType;
    use crate::discover::infer::InferredField;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn default_update() -> UpdateConfig {
        UpdateConfig { auto_build: true }
    }

    /// Helper to build a full MdvsToml with all sections present (auto_build=true).
    fn full_toml(fields: Vec<TomlField>) -> MdvsToml {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: default_update(),
            fields: FieldsConfig {
                ignore: vec![],
                field: fields,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        }
    }

    #[test]
    fn mdvs_toml_roundtrip() {
        let toml_doc = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: default_update(),
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![
                    TomlField {
                        name: "title".into(),
                        field_type: FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                    },
                    TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into(), "notes/**".into()],
                        required: vec!["blog/drafts/**".into(), "notes/**".into()],
                    },
                    TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                    },
                    TomlField {
                        name: "meta".into(),
                        field_type: FieldTypeSerde::Object {
                            object: BTreeMap::from([
                                ("author".into(), FieldTypeSerde::Scalar("String".into())),
                                ("version".into(), FieldTypeSerde::Scalar("Float".into())),
                            ]),
                        },
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                    },
                ],
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: Some("abc123".into()),
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };

        let toml_str = toml::to_string(&toml_doc).unwrap();
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, toml_doc);
    }

    #[test]
    fn parse_handwritten_mdvs_toml() {
        let handwritten = r#"
[scan]
glob = "blog/**"
include_bare_files = true

[update]
auto_build = true

[fields]
ignore = ["internal_id"]

[[fields.field]]
name = "title"
type = "String"
allowed = ["**"]
required = ["**"]

[[fields.field]]
name = "tags"
type = { array = "String" }
allowed = ["blog/**"]
required = []

[[fields.field]]
name = "meta"
type = { object = { author = "String", count = "Integer" } }
allowed = ["**"]
required = ["blog/**"]

[embedding_model]
name = "minishlab/potion-base-8M"

[chunking]
max_chunk_size = 1024

[search]
default_limit = 10
"#;

        let parsed: MdvsToml = toml::from_str(handwritten).unwrap();
        assert_eq!(parsed.scan.glob, "blog/**");
        assert!(parsed.scan.include_bare_files);
        assert_eq!(parsed.fields.ignore, vec!["internal_id"]);
        assert_eq!(parsed.fields.field.len(), 3);

        let title_ft = FieldType::try_from(&parsed.fields.field[0].field_type).unwrap();
        assert_eq!(title_ft, FieldType::String);

        let tags_ft = FieldType::try_from(&parsed.fields.field[1].field_type).unwrap();
        assert_eq!(tags_ft, FieldType::Array(Box::new(FieldType::String)));

        let meta_ft = FieldType::try_from(&parsed.fields.field[2].field_type).unwrap();
        assert_eq!(
            meta_ft,
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("count".into(), FieldType::Integer),
            ]))
        );
    }

    #[test]
    fn empty_fields_list_roundtrip() {
        let doc = full_toml(vec![]);
        let toml_str = toml::to_string(&doc).unwrap();
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.fields.field.len(), 0);
    }

    #[test]
    fn from_inferred_basic() {
        let schema = InferredSchema {
            fields: vec![
                InferredField {
                    name: "draft".into(),
                    field_type: FieldType::Boolean,
                    files: vec![PathBuf::from("blog/a.md")],
                    allowed: vec!["blog/**".into()],
                    required: vec!["blog/**".into()],
                },
                InferredField {
                    name: "tags".into(),
                    field_type: FieldType::Array(Box::new(FieldType::String)),
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["blog/**".into(), "notes/**".into()],
                    required: vec!["notes/**".into()],
                },
                InferredField {
                    name: "title".into(),
                    field_type: FieldType::String,
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                },
            ],
        };

        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let toml_doc =
            MdvsToml::from_inferred(&schema, scan, "minishlab/potion-base-8M", None, 1024, true);

        assert_eq!(toml_doc.scan.glob, "**");
        assert!(!toml_doc.scan.include_bare_files);
        assert_eq!(
            toml_doc.embedding_model.as_ref().unwrap().name,
            "minishlab/potion-base-8M"
        );
        assert_eq!(toml_doc.embedding_model.as_ref().unwrap().revision, None);
        assert_eq!(toml_doc.chunking.as_ref().unwrap().max_chunk_size, 1024);
        assert_eq!(toml_doc.fields.field.len(), 3);

        assert_eq!(toml_doc.fields.field[0].name, "draft");
        assert_eq!(
            FieldType::try_from(&toml_doc.fields.field[0].field_type).unwrap(),
            FieldType::Boolean
        );
        assert_eq!(toml_doc.fields.field[0].allowed, vec!["blog/**"]);
        assert_eq!(toml_doc.fields.field[0].required, vec!["blog/**"]);

        assert_eq!(toml_doc.fields.field[1].name, "tags");
        assert_eq!(
            FieldType::try_from(&toml_doc.fields.field[1].field_type).unwrap(),
            FieldType::Array(Box::new(FieldType::String))
        );

        assert_eq!(toml_doc.fields.field[2].name, "title");
    }

    #[test]
    fn from_inferred_empty() {
        let schema = InferredSchema { fields: vec![] };
        let scan = ScanConfig {
            glob: "docs/**".into(),
            include_bare_files: true,
            skip_gitignore: false,
        };
        let toml_doc = MdvsToml::from_inferred(
            &schema,
            scan,
            "minishlab/potion-base-8M",
            Some("rev123"),
            512,
            true,
        );
        assert_eq!(toml_doc.scan.glob, "docs/**");
        assert!(toml_doc.scan.include_bare_files);
        assert_eq!(
            toml_doc.embedding_model.as_ref().unwrap().revision,
            Some("rev123".into())
        );
        assert!(toml_doc.fields.field.is_empty());
    }

    #[test]
    fn from_inferred_no_auto_build() {
        let schema = InferredSchema { fields: vec![] };
        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let toml_doc = MdvsToml::from_inferred(
            &schema,
            scan,
            "minishlab/potion-base-8M",
            None,
            1024,
            false, // no auto_build
        );
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.chunking.is_none());
        assert!(toml_doc.search.is_none());
        assert!(!toml_doc.update.auto_build);
    }

    #[test]
    fn write_and_read_roundtrip() {
        let schema = InferredSchema {
            fields: vec![InferredField {
                name: "title".into(),
                field_type: FieldType::String,
                files: vec![PathBuf::from("a.md")],
                allowed: vec!["**".into()],
                required: vec!["**".into()],
            }],
        };
        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let toml_doc =
            MdvsToml::from_inferred(&schema, scan, "minishlab/potion-base-8M", None, 1024, true);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");

        toml_doc.write(&path).unwrap();
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, toml_doc);
    }

    #[test]
    fn write_uses_inline_tables_for_type() {
        let doc = full_toml(vec![
            TomlField {
                name: "tags".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("String".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
            },
            TomlField {
                name: "meta".into(),
                field_type: FieldTypeSerde::Object {
                    object: BTreeMap::from([(
                        "author".into(),
                        FieldTypeSerde::Scalar("String".into()),
                    )]),
                },
                allowed: vec!["**".into()],
                required: vec![],
            },
        ]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");
        doc.write(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();

        // Should use inline tables, not separate sections
        assert!(
            content.contains(r#"type = { array = "String" }"#),
            "expected inline array type, got:\n{content}"
        );
        assert!(
            content.contains(r#"type = { object = { author = "String" } }"#),
            "expected inline object type, got:\n{content}"
        );
        assert!(
            !content.contains("[fields.field.type]"),
            "should not have expanded type sections"
        );

        // Still roundtrips correctly
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn update_roundtrip() {
        let doc = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: true },
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
            },
            embedding_model: None,
            chunking: None,
            search: None,
            storage: None,
        };
        let toml_str = toml::to_string(&doc).unwrap();
        assert!(toml_str.contains("auto_build = true"));
        // Build sections should not appear
        assert!(!toml_str.contains("embedding_model"));
        assert!(!toml_str.contains("chunking"));
        assert!(!toml_str.contains("[search]"));
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert!(parsed.update.auto_build);
        assert!(parsed.embedding_model.is_none());

        // With auto_build false
        let doc2 = MdvsToml {
            update: UpdateConfig { auto_build: false },
            ..doc
        };
        let toml_str2 = toml::to_string(&doc2).unwrap();
        assert!(toml_str2.contains("auto_build = false"));
        let parsed2: MdvsToml = toml::from_str(&toml_str2).unwrap();
        assert!(!parsed2.update.auto_build);
    }

    #[test]
    fn validation_only_toml_roundtrip() {
        // Minimal toml with only validation sections (no build sections)
        let handwritten = r#"
[scan]
glob = "**"
include_bare_files = false

[update]
auto_build = false

[fields]
ignore = ["notes", "internal_id"]
"#;

        let parsed: MdvsToml = toml::from_str(handwritten).unwrap();
        assert_eq!(parsed.scan.glob, "**");
        assert!(!parsed.update.auto_build);
        assert_eq!(parsed.fields.ignore, vec!["notes", "internal_id"]);
        assert!(parsed.fields.field.is_empty());
        assert!(parsed.embedding_model.is_none());
        assert!(parsed.chunking.is_none());
        assert!(parsed.search.is_none());

        // Roundtrip
        let toml_str = toml::to_string(&parsed).unwrap();
        let reparsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(reparsed, parsed);
    }
}
