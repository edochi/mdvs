use crate::discover::infer::InferredSchema;
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tracing::instrument;

/// Placeholder for future update-specific settings.
/// Currently empty — `[update]` section is hidden from toml when default.
#[derive(Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct UpdateConfig {}

/// Check command settings (`[check]` in `mdvs.toml`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    /// Whether to auto-run update before validating.
    #[serde(default)]
    pub auto_update: bool,
}

/// Build workflow settings (`[build]` in `mdvs.toml`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuildConfig {
    /// Whether to auto-run update before building.
    #[serde(default)]
    pub auto_update: bool,
}

/// Search command settings (`[search]` in `mdvs.toml`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SearchConfig {
    /// Maximum number of results returned when `--limit` is not specified.
    pub default_limit: usize,
    /// Whether to auto-run update before building (when auto_build is true).
    #[serde(default)]
    pub auto_update: bool,
    /// Whether to auto-run build before searching.
    #[serde(default)]
    pub auto_build: bool,
    /// Prefix applied to internal column names in `--where` queries.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub internal_prefix: String,
    /// Per-column name overrides for internal columns in `--where` queries.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub aliases: HashMap<String, String>,
}

/// A single field definition in `[[fields.field]]`, specifying its type
/// and the glob patterns that constrain where it may or must appear.
///
/// All fields except `name` have permissive defaults: `type = "String"`,
/// `allowed = ["**"]`, `required = []`, `nullable = true`. A bare
/// `[[fields.field]]` with just a name is effectively unconstrained.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TomlField {
    /// Frontmatter key this definition applies to.
    pub name: String,
    /// Expected type (scalar, array, or object).
    #[serde(rename = "type", default = "default_field_type")]
    pub field_type: FieldTypeSerde,
    /// Glob patterns for files where this field is allowed.
    #[serde(default = "default_allowed")]
    pub allowed: Vec<String>,
    /// Glob patterns for files where this field is required.
    #[serde(default)]
    pub required: Vec<String>,
    /// Whether null values are accepted for this field.
    #[serde(default = "default_nullable")]
    pub nullable: bool,
}

fn default_field_type() -> FieldTypeSerde {
    FieldTypeSerde::Scalar("String".into())
}

fn default_allowed() -> Vec<String> {
    vec!["**".into()]
}

fn default_nullable() -> bool {
    true
}

/// The `[fields]` section: ignore list and per-field definitions.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
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
/// `check`, `fields`) are always present; build sections (`embedding_model`,
/// `chunking`, `build`, `search`) are optional and added by the first `build`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MdvsToml {
    /// File discovery settings (glob pattern, bare-file handling).
    pub scan: ScanConfig,
    /// Placeholder for future update-specific settings.
    #[serde(default, skip_serializing_if = "is_default_update_config")]
    pub update: UpdateConfig,
    /// Check command settings (auto-update).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check: Option<CheckConfig>,
    /// Embedding model identity. Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<EmbeddingModelConfig>,
    /// Chunk-size settings for semantic splitting. Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunking: Option<ChunkingConfig>,
    /// Build workflow settings (auto-update). Present only when build is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<BuildConfig>,
    /// Search defaults and auto-build/update settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search: Option<SearchConfig>,
    /// Field definitions and ignore list.
    pub fields: FieldsConfig,
}

fn is_default_update_config(c: &UpdateConfig) -> bool {
    *c == UpdateConfig::default()
}

impl MdvsToml {
    /// Build an `MdvsToml` from an inferred schema. Schema-only — no build sections.
    /// Build sections are added by the first `build` run.
    pub fn from_inferred(schema: &InferredSchema, scan: ScanConfig) -> Self {
        MdvsToml {
            scan,
            update: UpdateConfig::default(),
            check: Some(CheckConfig { auto_update: true }),
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
                        nullable: f.nullable,
                    })
                    .collect(),
            },
            embedding_model: None,
            chunking: None,
            build: Some(BuildConfig { auto_update: true }),
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: true,
                auto_build: true,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        }
    }

    /// Deserialize an `MdvsToml` from a file on disk.
    #[instrument(name = "read_config", skip_all, level = "debug")]
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: MdvsToml = toml::from_str(&content)?;
        Ok(config)
    }

    /// Validate config invariants that can be broken by manual edits.
    ///
    /// Three invariants are checked:
    /// 1. A field cannot appear in both `[fields].ignore` and `[[fields.field]]`.
    /// 2. All globs in `allowed` and `required` must end with `/*` or `/**`, or be
    ///    exactly `*` or `**`.
    /// 3. Every required glob must be covered by some allowed glob.
    pub fn validate(&self) -> anyhow::Result<()> {
        // Invariant 1: ignore and [[fields.field]] are mutually exclusive
        for ignored in &self.fields.ignore {
            if self.fields.field.iter().any(|f| &f.name == ignored) {
                anyhow::bail!(
                    "field '{}' appears in both [fields].ignore and [[fields.field]] — remove it from one",
                    ignored
                );
            }
        }

        for field in &self.fields.field {
            // Invariant 2: valid glob format
            for glob in field.allowed.iter().chain(field.required.iter()) {
                if !is_valid_glob_format(glob) {
                    anyhow::bail!(
                        "field '{}': invalid glob pattern '{}' — must end with /* or /** (or be * or **)",
                        field.name,
                        glob
                    );
                }
            }

            // Invariant 3: required ⊆ allowed
            for req in &field.required {
                if !glob_is_covered(req, &field.allowed) {
                    anyhow::bail!(
                        "field '{}': required glob '{}' is not covered by any allowed pattern",
                        field.name,
                        req
                    );
                }
            }
        }

        Ok(())
    }

    /// Serialize this config to TOML and write it to disk.
    /// Complex field types are post-processed into inline tables for readability.
    #[instrument(name = "write_config", skip_all, level = "debug")]
    pub fn write(&mut self, path: &Path) -> anyhow::Result<()> {
        self.fields.field.sort_by(|a, b| a.name.cmp(&b.name));
        let content = toml::to_string(self)?;
        let content = inline_field_types(&content)?;
        fs::write(path, content)?;
        Ok(())
    }
}

/// Check if a glob pattern has a valid format for allowed/required lists.
/// Valid: `*`, `**`, `path/*`, `path/**`. Invalid: bare paths, `*.md`, etc.
fn is_valid_glob_format(glob: &str) -> bool {
    glob == "*" || glob == "**" || glob.ends_with("/*") || glob.ends_with("/**")
}

/// Check if a required glob is covered by any allowed glob.
///
/// A required glob `R` is covered by an allowed glob `A` if every path matching `R`
/// also matches `A`. We check this by stripping the `/*` or `/**` suffix from `R`
/// to get its directory path, then testing if that path matches any `A` via globset.
fn glob_is_covered(required: &str, allowed: &[String]) -> bool {
    for allowed_glob in allowed {
        // "**" covers everything
        if allowed_glob == "**" {
            return true;
        }

        // Exact match (most common case from inference)
        if allowed_glob == required {
            return true;
        }

        // "*" covers "*" (root-level shallow)
        if allowed_glob == "*" && required == "*" {
            return true;
        }

        // Glob-based containment: strip suffix from required and test against allowed
        let req_dir = strip_glob_suffix(required);
        if !req_dir.is_empty()
            && let Ok(glob) = globset::Glob::new(allowed_glob)
        {
            let matcher = glob.compile_matcher();
            if matcher.is_match(req_dir) {
                return true;
            }
        }
    }

    false
}

/// Strip the `/*` or `/**` suffix from a glob to get the directory path.
/// Returns "" for root-level globs (`*`, `**`).
fn strip_glob_suffix(glob: &str) -> &str {
    if glob == "*" || glob == "**" {
        ""
    } else if let Some(dir) = glob.strip_suffix("/**") {
        dir
    } else if let Some(dir) = glob.strip_suffix("/*") {
        dir
    } else {
        glob
    }
}

/// Post-process serialized TOML to render `type` fields as inline tables.
/// The `toml` crate expands them into `[fields.field.type]` sections;
/// we convert those back to `type = { array = "String" }` form.
fn inline_field_types(toml_str: &str) -> anyhow::Result<String> {
    let mut doc = toml_str.parse::<toml_edit::DocumentMut>()?;

    if let Some(fields) = doc.get_mut("fields")
        && let Some(field_array) = fields.get_mut("field")
        && let Some(array_of_tables) = field_array.as_array_of_tables_mut()
    {
        for table in array_of_tables.iter_mut() {
            if let Some(type_item) = table.get_mut("type")
                && let Some(t) = type_item.as_table_mut()
            {
                let inline = t.clone().into_inline_table();
                *type_item = toml_edit::Item::Value(inline.into());
            }
            // Normalize key formatting (fix missing space before `=`)
            if let Some(mut key) = table.key_mut("type") {
                key.fmt();
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
        UpdateConfig {}
    }

    /// Helper to build a full MdvsToml with all sections present.
    fn full_toml(fields: Vec<TomlField>) -> MdvsToml {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: default_update(),
            check: None,
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
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
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
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![
                    TomlField {
                        name: "title".into(),
                        field_type: FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                        nullable: false,
                    },
                    TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into(), "notes/**".into()],
                        required: vec!["blog/drafts/**".into(), "notes/**".into()],
                        nullable: false,
                    },
                    TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                        nullable: false,
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
                        nullable: false,
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
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
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
                    nullable: false,
                },
                InferredField {
                    name: "tags".into(),
                    field_type: FieldType::Array(Box::new(FieldType::String)),
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["blog/**".into(), "notes/**".into()],
                    required: vec!["notes/**".into()],
                    nullable: false,
                },
                InferredField {
                    name: "title".into(),
                    field_type: FieldType::String,
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                    nullable: false,
                },
            ],
        };

        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let toml_doc = MdvsToml::from_inferred(&schema, scan);

        assert_eq!(toml_doc.scan.glob, "**");
        assert!(!toml_doc.scan.include_bare_files);
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.chunking.is_none());
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
        let toml_doc = MdvsToml::from_inferred(&schema, scan);
        assert_eq!(toml_doc.scan.glob, "docs/**");
        assert!(toml_doc.scan.include_bare_files);
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.fields.field.is_empty());
    }

    #[test]
    fn from_inferred_schema_only() {
        let schema = InferredSchema { fields: vec![] };
        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let toml_doc = MdvsToml::from_inferred(&schema, scan);
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.chunking.is_none());
        assert!(toml_doc.build.is_some());
        assert!(toml_doc.search.is_some());
        assert!(toml_doc.search.as_ref().unwrap().auto_build);
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
                nullable: false,
            }],
        };
        let scan = ScanConfig {
            glob: "**".into(),
            include_bare_files: false,
            skip_gitignore: false,
        };
        let mut toml_doc = MdvsToml::from_inferred(&schema, scan);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");

        toml_doc.write(&path).unwrap();
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, toml_doc);
    }

    #[test]
    fn write_uses_inline_tables_for_type() {
        let mut doc = full_toml(vec![
            TomlField {
                name: "tags".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("String".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
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
                nullable: false,
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
    fn validation_only_roundtrip() {
        let doc = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        let toml_str = toml::to_string(&doc).unwrap();
        // Build sections should not appear
        assert!(!toml_str.contains("embedding_model"));
        assert!(!toml_str.contains("chunking"));
        assert!(!toml_str.contains("[search]"));
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert!(parsed.embedding_model.is_none());
    }

    #[test]
    fn validation_only_toml_roundtrip_handwritten() {
        // Minimal toml with only validation sections (no build sections)
        let handwritten = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]
ignore = ["notes", "internal_id"]
"#;

        let parsed: MdvsToml = toml::from_str(handwritten).unwrap();
        assert_eq!(parsed.scan.glob, "**");
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

    #[test]
    fn bare_field_definition_defaults() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]

[[fields.field]]
name = "title"
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.fields.field.len(), 1);

        let f = &parsed.fields.field[0];
        assert_eq!(f.name, "title");
        assert_eq!(f.field_type, FieldTypeSerde::Scalar("String".into()));
        assert_eq!(f.allowed, vec!["**"]);
        assert!(f.required.is_empty());
        assert!(f.nullable);
    }

    #[test]
    fn bare_field_partial_override() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]

[[fields.field]]
name = "draft"
type = "Boolean"
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        let f = &parsed.fields.field[0];
        assert_eq!(f.name, "draft");
        assert_eq!(f.field_type, FieldTypeSerde::Scalar("Boolean".into()));
        assert_eq!(f.allowed, vec!["**"]);
        assert!(f.required.is_empty());
        assert!(f.nullable);
    }

    #[test]
    fn bare_field_with_explicit_non_defaults() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]

[[fields.field]]
name = "tags"
type = { array = "String" }
allowed = ["blog/**"]
required = ["blog/**"]
nullable = false
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        let f = &parsed.fields.field[0];
        assert_eq!(f.name, "tags");
        assert_eq!(
            f.field_type,
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into()))
            }
        );
        assert_eq!(f.allowed, vec!["blog/**"]);
        assert_eq!(f.required, vec!["blog/**"]);
        assert!(!f.nullable);
    }

    // --- Invariant 1: ignore vs field mutual exclusion ---

    #[test]
    fn validate_ignore_and_field_conflict() {
        let mut config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: true,
        }]);
        config.fields.ignore = vec!["tags".into()];
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("appears in both [fields].ignore and [[fields.field]]"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_ignore_no_conflict_passes() {
        let mut config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: true,
        }]);
        config.fields.ignore = vec!["other_field".into()];
        assert!(config.validate().is_ok());
    }

    // --- Invariant 2: valid glob format ---

    #[test]
    fn validate_invalid_glob_format_bare_path() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["blog".into()],
            required: vec![],
            nullable: false,
        }]);
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string().contains("invalid glob pattern 'blog'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_invalid_glob_format_file_pattern() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec!["blog/post.md".into()],
            nullable: false,
        }]);
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("invalid glob pattern 'blog/post.md'"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_valid_glob_formats() {
        let config = full_toml(vec![
            TomlField {
                name: "a".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec!["*".into()],
                nullable: false,
            },
            TomlField {
                name: "b".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["blog/**".into()],
                required: vec!["blog/**".into()],
                nullable: false,
            },
            TomlField {
                name: "c".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["people/*".into()],
                required: vec![],
                nullable: false,
            },
        ]);
        assert!(config.validate().is_ok());
    }

    // --- Invariant 3: required ⊆ allowed ---

    #[test]
    fn validate_required_not_covered_by_allowed() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["notes/**".into()],
            required: vec!["blog/**".into()],
            nullable: false,
        }]);
        let err = config.validate().unwrap_err();
        assert!(
            err.to_string()
                .contains("required glob 'blog/**' is not covered by any allowed pattern"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_wildcard_allowed_covers_any_required() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec!["blog/**".into()],
            nullable: false,
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_parent_glob_covers_child_required() {
        let config = full_toml(vec![TomlField {
            name: "action_items".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["meetings/**".into()],
            required: vec!["meetings/all-hands/**".into()],
            nullable: false,
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_exact_match_required_allowed() {
        let config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["blog/**".into(), "notes/**".into()],
            required: vec!["blog/**".into()],
            nullable: false,
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_empty_required_passes() {
        let config = full_toml(vec![TomlField {
            name: "draft".into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_valid_config_passes() {
        let config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["blog/**".into(), "notes/**".into()],
            required: vec!["blog/**".into()],
            nullable: false,
        }]);
        assert!(config.validate().is_ok());
    }

    // --- deny_unknown_fields tests ---

    #[test]
    fn unknown_field_in_scan_rejected() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = true
glob_pattern = "*.md"

[fields]
"#;
        let err = toml::from_str::<MdvsToml>(toml_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected unknown field error: {msg}"
        );
    }

    #[test]
    fn unknown_field_in_update_rejected() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = true

[update]
auto_build = true

[fields]
"#;
        let err = toml::from_str::<MdvsToml>(toml_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected unknown field error: {msg}"
        );
    }

    #[test]
    fn unknown_field_in_fields_field_rejected() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = true

[fields]

[[fields.field]]
name = "title"
types = "String"
"#;
        let err = toml::from_str::<MdvsToml>(toml_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected unknown field error: {msg}"
        );
    }

    #[test]
    fn unknown_top_level_section_rejected() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = true

[storage]
internal_prefix = "_"

[fields]
"#;
        let err = toml::from_str::<MdvsToml>(toml_str).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown field"),
            "expected unknown field error: {msg}"
        );
    }

    #[test]
    fn valid_config_still_parses_with_deny_unknown() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false
skip_gitignore = true

[embedding_model]
provider = "model2vec"
name = "minishlab/potion-base-8M"

[chunking]
max_chunk_size = 1024

[search]
default_limit = 10

[fields]
ignore = []

[[fields.field]]
name = "title"
type = "String"
allowed = ["**"]
required = ["**"]
nullable = false
"#;
        let config: MdvsToml = toml::from_str(toml_str).unwrap();
        assert_eq!(config.scan.glob, "**");
        assert_eq!(config.fields.field[0].name, "title");
    }
}
