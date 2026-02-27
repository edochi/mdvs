use crate::discover::infer::InferredSchema;
use crate::schema::shared::{FieldTypeSerde, TomlConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

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
    pub fields: Vec<TomlField>,
}

impl MdvsToml {
    pub fn from_inferred(schema: &InferredSchema, glob: &str, include_bare_files: bool) -> Self {
        MdvsToml {
            config: TomlConfig {
                glob: glob.to_string(),
                include_bare_files,
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
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn mdvs_toml_roundtrip() {
        let toml_doc = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
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

        let toml_doc = MdvsToml::from_inferred(&schema, "**", false);

        assert_eq!(toml_doc.config.glob, "**");
        assert!(!toml_doc.config.include_bare_files);
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
        let toml_doc = MdvsToml::from_inferred(&schema, "docs/**", true);
        assert_eq!(toml_doc.config.glob, "docs/**");
        assert!(toml_doc.config.include_bare_files);
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
        let toml_doc = MdvsToml::from_inferred(&schema, "**", false);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");

        toml_doc.write(&path).unwrap();
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, toml_doc);
    }
}
