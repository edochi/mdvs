use crate::discover::infer::InferredSchema;
use crate::schema::shared::{ChunkingConfig, FieldTypeSerde, ModelInfo, TomlConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize, PartialEq, Clone)]
#[serde(rename_all = "lowercase")]
pub enum OnError {
    Fail,
    Skip,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkflowConfig {
    pub auto_build: bool,
    pub on_error: OnError,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TomlField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldTypeSerde,
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MdvsToml {
    pub config: TomlConfig,
    pub model: ModelInfo,
    pub chunking: ChunkingConfig,
    pub workflow: WorkflowConfig,
    pub fields: Vec<TomlField>,
}

impl MdvsToml {
    pub fn from_inferred(
        schema: &InferredSchema,
        glob: &str,
        include_bare_files: bool,
        model_name: &str,
        model_revision: Option<&str>,
        max_chunk_size: usize,
        auto_build: bool,
    ) -> Self {
        MdvsToml {
            config: TomlConfig {
                glob: glob.to_string(),
                include_bare_files,
            },
            model: ModelInfo {
                name: model_name.to_string(),
                revision: model_revision.map(|s| s.to_string()),
            },
            chunking: ChunkingConfig { max_chunk_size },
            workflow: WorkflowConfig {
                auto_build,
                on_error: OnError::Fail,
            },
            fields: schema
                .fields
                .iter()
                .map(|f| TomlField {
                    name: f.name.clone(),
                    field_type: FieldTypeSerde::from(&f.field_type),
                    allowed: f.allowed.clone(),
                    required: f.required.clone(),
                })
                .collect(),
        }
    }

    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: MdvsToml = toml::from_str(&content)?;
        Ok(config)
    }

    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::field_type::FieldType;
    use crate::discover::infer::InferredField;
    use crate::schema::shared::{ChunkingConfig, ModelInfo};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn default_workflow() -> WorkflowConfig {
        WorkflowConfig {
            auto_build: true,
            on_error: OnError::Fail,
        }
    }

    #[test]
    fn mdvs_toml_roundtrip() {
        let toml_doc = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: "minishlab/potion-base-8M".into(),
                revision: Some("abc123".into()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            workflow: default_workflow(),
            fields: vec![
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
        };

        let toml_str = toml::to_string(&toml_doc).unwrap();
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, toml_doc);
    }

    #[test]
    fn parse_handwritten_mdvs_toml() {
        let handwritten = r#"
[config]
glob = "blog/**"
include_bare_files = true

[model]
name = "minishlab/potion-base-8M"

[chunking]
max_chunk_size = 1024

[workflow]
auto_build = true
on_error = "fail"

[[fields]]
name = "title"
type = "String"
allowed = ["**"]
required = ["**"]

[[fields]]
name = "tags"
type = { array = "String" }
allowed = ["blog/**"]
required = []

[[fields]]
name = "meta"
type = { object = { author = "String", count = "Integer" } }
allowed = ["**"]
required = ["blog/**"]
"#;

        let parsed: MdvsToml = toml::from_str(handwritten).unwrap();
        assert_eq!(parsed.config.glob, "blog/**");
        assert!(parsed.config.include_bare_files);
        assert_eq!(parsed.fields.len(), 3);

        let title_ft = FieldType::try_from(&parsed.fields[0].field_type).unwrap();
        assert_eq!(title_ft, FieldType::String);

        let tags_ft = FieldType::try_from(&parsed.fields[1].field_type).unwrap();
        assert_eq!(tags_ft, FieldType::Array(Box::new(FieldType::String)));

        let meta_ft = FieldType::try_from(&parsed.fields[2].field_type).unwrap();
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
        let doc = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            workflow: default_workflow(),
            fields: vec![],
        };
        let toml_str = toml::to_string(&doc).unwrap();
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.fields.len(), 0);
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
                    files: vec![
                        PathBuf::from("blog/a.md"),
                        PathBuf::from("notes/b.md"),
                    ],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                },
            ],
        };

        let toml_doc =
            MdvsToml::from_inferred(&schema, "**", false, "minishlab/potion-base-8M", None, 1024, true);

        assert_eq!(toml_doc.config.glob, "**");
        assert!(!toml_doc.config.include_bare_files);
        assert_eq!(toml_doc.model.name, "minishlab/potion-base-8M");
        assert_eq!(toml_doc.model.revision, None);
        assert_eq!(toml_doc.chunking.max_chunk_size, 1024);
        assert_eq!(toml_doc.fields.len(), 3);

        assert_eq!(toml_doc.fields[0].name, "draft");
        assert_eq!(
            FieldType::try_from(&toml_doc.fields[0].field_type).unwrap(),
            FieldType::Boolean
        );
        assert_eq!(toml_doc.fields[0].allowed, vec!["blog/**"]);
        assert_eq!(toml_doc.fields[0].required, vec!["blog/**"]);

        assert_eq!(toml_doc.fields[1].name, "tags");
        assert_eq!(
            FieldType::try_from(&toml_doc.fields[1].field_type).unwrap(),
            FieldType::Array(Box::new(FieldType::String))
        );

        assert_eq!(toml_doc.fields[2].name, "title");
    }

    #[test]
    fn from_inferred_empty() {
        let schema = InferredSchema { fields: vec![] };
        let toml_doc = MdvsToml::from_inferred(
            &schema,
            "docs/**",
            true,
            "minishlab/potion-base-8M",
            Some("rev123"),
            512,
            true,
        );
        assert_eq!(toml_doc.config.glob, "docs/**");
        assert!(toml_doc.config.include_bare_files);
        assert_eq!(toml_doc.model.revision, Some("rev123".into()));
        assert!(toml_doc.fields.is_empty());
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
        let toml_doc =
            MdvsToml::from_inferred(&schema, "**", false, "minishlab/potion-base-8M", None, 1024, true);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");

        toml_doc.write(&path).unwrap();
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, toml_doc);
    }

    #[test]
    fn workflow_roundtrip() {
        // Test OnError::Fail
        let doc_fail = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: "test-model".into(),
                revision: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            workflow: WorkflowConfig {
                auto_build: true,
                on_error: OnError::Fail,
            },
            fields: vec![],
        };
        let toml_str = toml::to_string(&doc_fail).unwrap();
        assert!(toml_str.contains("on_error = \"fail\""));
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.workflow.on_error, OnError::Fail);
        assert!(parsed.workflow.auto_build);

        // Test OnError::Skip + auto_build false
        let doc_skip = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: "test-model".into(),
                revision: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            workflow: WorkflowConfig {
                auto_build: false,
                on_error: OnError::Skip,
            },
            fields: vec![],
        };
        let toml_str = toml::to_string(&doc_skip).unwrap();
        assert!(toml_str.contains("on_error = \"skip\""));
        assert!(toml_str.contains("auto_build = false"));
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.workflow.on_error, OnError::Skip);
        assert!(!parsed.workflow.auto_build);
    }
}
