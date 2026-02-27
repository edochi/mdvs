#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! serde = { version = "1", features = ["derive"] }
//! serde_json = "1"
//! toml = "0.8"
//! ```

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ============================================================================
// FieldType — recursive enum with TOML serialization
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
enum FieldType {
    Boolean,
    Integer,
    Float,
    String,
    Array(Box<FieldType>),
    Object(BTreeMap<std::string::String, FieldType>),
}

/// Serde-friendly representation of FieldType for TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
enum FieldTypeSerde {
    Scalar(std::string::String),
    Array {
        array: Box<FieldTypeSerde>,
    },
    Object {
        object: BTreeMap<std::string::String, FieldTypeSerde>,
    },
}

impl From<&FieldType> for FieldTypeSerde {
    fn from(ft: &FieldType) -> Self {
        match ft {
            FieldType::Boolean => FieldTypeSerde::Scalar("Boolean".into()),
            FieldType::Integer => FieldTypeSerde::Scalar("Integer".into()),
            FieldType::Float => FieldTypeSerde::Scalar("Float".into()),
            FieldType::String => FieldTypeSerde::Scalar("String".into()),
            FieldType::Array(inner) => FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::from(inner.as_ref())),
            },
            FieldType::Object(fields) => FieldTypeSerde::Object {
                object: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), FieldTypeSerde::from(v)))
                    .collect(),
            },
        }
    }
}

impl TryFrom<&FieldTypeSerde> for FieldType {
    type Error = std::string::String;

    fn try_from(s: &FieldTypeSerde) -> Result<Self, Self::Error> {
        match s {
            FieldTypeSerde::Scalar(name) => match name.as_str() {
                "Boolean" => Ok(FieldType::Boolean),
                "Integer" => Ok(FieldType::Integer),
                "Float" => Ok(FieldType::Float),
                "String" => Ok(FieldType::String),
                other => Err(format!("unknown type: {other}")),
            },
            FieldTypeSerde::Array { array } => {
                let inner = FieldType::try_from(array.as_ref())?;
                Ok(FieldType::Array(Box::new(inner)))
            }
            FieldTypeSerde::Object { object } => {
                let mut fields = BTreeMap::new();
                for (k, v) in object {
                    fields.insert(k.clone(), FieldType::try_from(v)?);
                }
                Ok(FieldType::Object(fields))
            }
        }
    }
}

// ============================================================================
// mdvs.toml — config + fields (user-editable boundaries)
// ============================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TomlConfig {
    glob: std::string::String,
    include_bare_files: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TomlField {
    name: std::string::String,
    #[serde(rename = "type")]
    field_type: FieldTypeSerde,
    allowed: Vec<std::string::String>,
    required: Vec<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct MdvsToml {
    config: TomlConfig,
    fields: Vec<TomlField>,
}

// ============================================================================
// mdvs.lock — config + files + fields (observed state)
// ============================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct LockFile {
    path: std::string::String,
    content_hash: std::string::String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct LockField {
    name: std::string::String,
    #[serde(rename = "type")]
    field_type: FieldTypeSerde,
    files: Vec<std::string::String>,
    allowed: Vec<std::string::String>,
    required: Vec<std::string::String>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct MdvsLock {
    config: TomlConfig,
    files: Vec<LockFile>,
    fields: Vec<LockField>,
}

// ============================================================================
// Tests
// ============================================================================

/// Wrapper for testing FieldTypeSerde in isolation (TOML needs a root table).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct TypeWrapper {
    #[serde(rename = "type")]
    field_type: FieldTypeSerde,
}

fn wrap(ft: &FieldType) -> TypeWrapper {
    TypeWrapper {
        field_type: FieldTypeSerde::from(ft),
    }
}

fn main() {
    println!("=== TOML serialization tests ===\n");

    // --- Test 1: FieldType roundtrip — all scalar types ---
    {
        let types = vec![
            FieldType::Boolean,
            FieldType::Integer,
            FieldType::Float,
            FieldType::String,
        ];
        for ft in &types {
            let w = wrap(ft);
            let toml_str = toml::to_string(&w).unwrap();
            let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
            let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
            assert_eq!(&roundtripped, ft);
        }
        println!("  1. Scalar FieldType roundtrip (Boolean, Integer, Float, String)  ✓");
    }

    // --- Test 2: Array(String) roundtrip ---
    {
        let ft = FieldType::Array(Box::new(FieldType::String));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains("array"));
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
        println!("  2. Array(String) roundtrip  ✓");
    }

    // --- Test 3: Array(Array(Float)) roundtrip ---
    {
        let ft = FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float))));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
        println!("  3. Array(Array(Float)) roundtrip  ✓");
    }

    // --- Test 4: Object roundtrip ---
    {
        let ft = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("version".into(), FieldType::Float),
            ("draft".into(), FieldType::Boolean),
        ]));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains("object"));
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
        println!("  4. Object(author:Str, version:Float, draft:Bool) roundtrip  ✓");
    }

    // --- Test 5: Object with nested Array roundtrip ---
    {
        let ft = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
        ]));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
        println!("  5. Object with nested Array roundtrip  ✓");
    }

    // --- Test 6: Unknown scalar type → error ---
    {
        let bad = FieldTypeSerde::Scalar("Date".into());
        let result = FieldType::try_from(&bad);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown type"));
        println!("  6. Unknown scalar type → error  ✓");
    }

    // --- Test 7: mdvs.toml roundtrip ---
    {
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
        println!("  --- mdvs.toml output ---");
        for line in toml_str.lines() {
            println!("  | {line}");
        }
        println!("  --- end ---");

        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, toml_doc);
        println!("  7. mdvs.toml roundtrip  ✓");
    }

    // --- Test 8: mdvs.lock roundtrip ---
    {
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
        println!("  --- mdvs.lock output ---");
        for line in toml_str.lines() {
            println!("  | {line}");
        }
        println!("  --- end ---");

        let parsed: MdvsLock = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, lock_doc);
        println!("  8. mdvs.lock roundtrip  ✓");
    }

    // --- Test 9: Parse handwritten mdvs.toml ---
    {
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
        assert_eq!(parsed.config.include_bare_files, true);
        assert_eq!(parsed.fields.len(), 3);

        // Verify types parsed correctly
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

        println!("  9. Parse handwritten mdvs.toml  ✓");
    }

    // --- Test 10: Parse handwritten mdvs.lock ---
    {
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

        println!("  10. Parse handwritten mdvs.lock  ✓");
    }

    // --- Test 11: Empty fields list ---
    {
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
        println!("  11. Empty fields list roundtrip  ✓");
    }

    // --- Test 12: Deeply nested type ---
    {
        // Array(Object(tags: Array(String), meta: Object(x: Integer)))
        let ft = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            (
                "meta".into(),
                FieldType::Object(BTreeMap::from([("x".into(), FieldType::Integer)])),
            ),
        ]))));
        let serde = FieldTypeSerde::from(&ft);
        let toml_str = toml::to_string(&serde).unwrap();
        let parsed: FieldTypeSerde = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed).unwrap();
        assert_eq!(roundtripped, ft);
        println!("  12. Deeply nested type roundtrip  ✓");
    }

    println!("\n=== All tests passed ===");
}
