use crate::schema::shared::{FieldTypeSerde, TomlConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LockFile {
    pub path: String,
    pub content_hash: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LockField {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: FieldTypeSerde,
    pub files: Vec<String>,
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct MdvsLock {
    pub config: TomlConfig,
    pub files: Vec<LockFile>,
    pub fields: Vec<LockField>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdvs_lock_roundtrip() {
        let lock_doc = MdvsLock {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            files: vec![
                LockFile {
                    path: "blog/post1.md".into(),
                    content_hash: "a1b2c3d4".into(),
                },
                LockFile {
                    path: "blog/post2.md".into(),
                    content_hash: "e5f6g7h8".into(),
                },
                LockFile {
                    path: "notes/idea.md".into(),
                    content_hash: "i9j0k1l2".into(),
                },
            ],
            fields: vec![
                LockField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    files: vec![
                        "blog/post1.md".into(),
                        "blog/post2.md".into(),
                        "notes/idea.md".into(),
                    ],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                },
                LockField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    files: vec!["blog/post1.md".into()],
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                },
            ],
        };

        let toml_str = toml::to_string(&lock_doc).unwrap();
        let parsed: MdvsLock = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, lock_doc);
    }

    #[test]
    fn parse_handwritten_mdvs_lock() {
        let handwritten = r#"
[config]
glob = "**"
include_bare_files = false

[[files]]
path = "blog/hello.md"
content_hash = "sha256abc"

[[files]]
path = "notes/idea.md"
content_hash = "sha256def"

[[fields]]
name = "title"
type = "String"
files = ["blog/hello.md", "notes/idea.md"]
allowed = ["**"]
required = ["**"]

[[fields]]
name = "draft"
type = "Boolean"
files = ["blog/hello.md"]
allowed = ["blog/**"]
required = ["blog/**"]
"#;

        let parsed: MdvsLock = toml::from_str(handwritten).unwrap();
        assert_eq!(parsed.files.len(), 2);
        assert_eq!(parsed.files[0].path, "blog/hello.md");
        assert_eq!(parsed.fields.len(), 2);
        assert_eq!(parsed.fields[1].files, vec!["blog/hello.md"]);
    }
}
