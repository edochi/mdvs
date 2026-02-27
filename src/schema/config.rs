use crate::schema::shared::{FieldTypeSerde, TomlConfig};
use serde::{Deserialize, Serialize};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::field_type::FieldType;
    use std::collections::BTreeMap;

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
}
