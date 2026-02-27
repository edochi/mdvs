use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::schema::shared::{FieldTypeSerde, ModelInfo, TomlConfig};
use serde::{Deserialize, Serialize};
use std::fs;
use std::hash::{DefaultHasher, Hasher};
use std::path::Path;

fn content_hash(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    hasher.write(content.as_bytes());
    format!("{:016x}", hasher.finish())
}

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
    pub model: ModelInfo,
    pub files: Vec<LockFile>,
    pub fields: Vec<LockField>,
}

impl MdvsLock {
    pub fn from_inferred(
        schema: &InferredSchema,
        scanned: &ScannedFiles,
        glob: &str,
        include_bare_files: bool,
        model_name: &str,
        model_revision: &str,
    ) -> Self {
        MdvsLock {
            config: TomlConfig {
                glob: glob.to_string(),
                include_bare_files,
            },
            model: ModelInfo {
                name: model_name.to_string(),
                revision: Some(model_revision.to_string()),
            },
            files: scanned
                .files
                .iter()
                .map(|f| LockFile {
                    path: f.path.display().to_string(),
                    content_hash: content_hash(&f.content),
                })
                .collect(),
            fields: schema
                .fields
                .iter()
                .map(|f| LockField {
                    name: f.name.clone(),
                    field_type: FieldTypeSerde::from(&f.field_type),
                    files: f.files.iter().map(|p| p.display().to_string()).collect(),
                    allowed: f.allowed.clone(),
                    required: f.required.clone(),
                })
                .collect(),
        }
    }

    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let lock: MdvsLock = toml::from_str(&content)?;
        Ok(lock)
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
    use crate::discover::scan::ScannedFile;
    use crate::schema::shared::ModelInfo;
    use std::path::PathBuf;

    #[test]
    fn mdvs_lock_roundtrip() {
        let lock_doc = MdvsLock {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: "minishlab/potion-base-8M".into(),
                revision: Some("abc123".into()),
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

[model]
name = "minishlab/potion-base-8M"
revision = "abc123"

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

    #[test]
    fn from_inferred_basic() {
        let scanned = ScannedFiles {
            files: vec![
                ScannedFile {
                    path: PathBuf::from("blog/a.md"),
                    data: None,
                    content: "# Hello\nBody text.".into(),
                },
                ScannedFile {
                    path: PathBuf::from("notes/b.md"),
                    data: None,
                    content: "# Notes\nSome notes.".into(),
                },
            ],
        };
        let schema = InferredSchema {
            fields: vec![
                InferredField {
                    name: "title".into(),
                    field_type: FieldType::String,
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                },
                InferredField {
                    name: "tags".into(),
                    field_type: FieldType::Array(Box::new(FieldType::String)),
                    files: vec![PathBuf::from("blog/a.md")],
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                },
            ],
        };

        let lock = MdvsLock::from_inferred(
            &schema,
            &scanned,
            "**",
            false,
            "minishlab/potion-base-8M",
            "abc123",
        );

        assert_eq!(lock.config.glob, "**");
        assert!(!lock.config.include_bare_files);
        assert_eq!(lock.model.name, "minishlab/potion-base-8M");
        assert_eq!(lock.model.revision, Some("abc123".into()));

        assert_eq!(lock.files.len(), 2);
        assert_eq!(lock.files[0].path, "blog/a.md");
        assert!(!lock.files[0].content_hash.is_empty());
        assert_eq!(lock.files[1].path, "notes/b.md");

        assert_eq!(lock.fields.len(), 2);
        assert_eq!(lock.fields[0].name, "title");
        assert_eq!(
            lock.fields[0].files,
            vec!["blog/a.md", "notes/b.md"]
        );
        assert_eq!(lock.fields[0].allowed, vec!["**"]);
        assert_eq!(lock.fields[0].required, vec!["**"]);

        assert_eq!(lock.fields[1].name, "tags");
        assert_eq!(lock.fields[1].files, vec!["blog/a.md"]);
    }

    #[test]
    fn from_inferred_empty() {
        let scanned = ScannedFiles { files: vec![] };
        let schema = InferredSchema { fields: vec![] };
        let lock =
            MdvsLock::from_inferred(&schema, &scanned, "**", false, "test/model", "rev1");
        assert!(lock.files.is_empty());
        assert!(lock.fields.is_empty());
    }

    #[test]
    fn content_hash_deterministic() {
        let hash1 = content_hash("Hello, world!");
        let hash2 = content_hash("Hello, world!");
        assert_eq!(hash1, hash2);

        let hash3 = content_hash("Different content");
        assert_ne!(hash1, hash3);

        assert_eq!(hash1.len(), 16);
    }

    #[test]
    fn write_and_read_roundtrip() {
        let scanned = ScannedFiles {
            files: vec![ScannedFile {
                path: PathBuf::from("a.md"),
                data: None,
                content: "# Test".into(),
            }],
        };
        let schema = InferredSchema {
            fields: vec![InferredField {
                name: "title".into(),
                field_type: FieldType::String,
                files: vec![PathBuf::from("a.md")],
                allowed: vec!["**".into()],
                required: vec!["**".into()],
            }],
        };
        let lock =
            MdvsLock::from_inferred(&schema, &scanned, "**", false, "test/model", "rev1");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.lock");

        lock.write(&path).unwrap();
        let loaded = MdvsLock::read(&path).unwrap();
        assert_eq!(loaded, lock);
    }
}
