use crate::discover::field_type::FieldType;
use crate::discover::infer::{InferredSchema, infer_constraints};
use crate::preprocess::ValueStage;
use crate::schema::constraints::Constraints;
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
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
    /// Optional value constraints (categories, range, length, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraints: Option<Constraints>,
    /// Stage-2 preprocessors to apply before validation. Auto-populated by
    /// inference when type-widening events are observed; can also be set
    /// manually. Empty means strict validation (no coercion).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preprocess: Vec<ValueStage>,
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

/// The `[fields]` section: ignore list, per-field definitions, and inference thresholds.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FieldsConfig {
    /// Field names to ignore during validation (known but unconstrained).
    #[serde(default)]
    pub ignore: Vec<String>,
    /// Constrained field definitions (`[[fields.field]]` entries).
    #[serde(default, rename = "field")]
    pub field: Vec<TomlField>,
    /// Maximum distinct values for a field to be auto-inferred as categorical.
    #[serde(
        default = "default_max_categories",
        skip_serializing_if = "is_default_max_categories"
    )]
    pub max_categories: usize,
    /// Minimum average repetition (occurrences / distinct) for categorical inference.
    #[serde(
        default = "default_min_category_repetition",
        skip_serializing_if = "is_default_min_category_repetition"
    )]
    pub min_category_repetition: usize,
}

fn default_max_categories() -> usize {
    10
}

fn is_default_max_categories(v: &usize) -> bool {
    *v == default_max_categories()
}

fn default_min_category_repetition() -> usize {
    3
}

fn is_default_min_category_repetition(v: &usize) -> bool {
    *v == default_min_category_repetition()
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
    /// Build a default `MdvsToml` around a field list. Used by `check --schema`
    /// when no `mdvs.toml` exists at the target path — the schema provides the
    /// fields; everything else takes its default value. Matches the shape
    /// produced by a fresh `init --suppress-auto-build`.
    pub fn default_with_fields(fields: Vec<TomlField>, ignore: Vec<String>) -> Self {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig::default(),
            check: None,
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
            fields: FieldsConfig {
                ignore,
                field: fields,
                max_categories: default_max_categories(),
                min_category_repetition: default_min_category_repetition(),
            },
        }
    }

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
                    .map(|f| {
                        let max_cat = default_max_categories();
                        let min_rep = default_min_category_repetition();
                        TomlField {
                            name: f.name.clone(),
                            field_type: FieldTypeSerde::from(&f.field_type),
                            allowed: f.allowed.clone(),
                            required: f.required.clone(),
                            nullable: f.nullable,
                            constraints: infer_constraints(f, max_cat, min_rep),
                            preprocess: f.preprocess.clone(),
                        }
                    })
                    .collect(),
                max_categories: default_max_categories(),
                min_category_repetition: default_min_category_repetition(),
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
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Distinguish "directory doesn't exist" from "mdvs.toml doesn't exist"
                let parent = path.parent().unwrap_or(Path::new("."));
                if !parent.exists() {
                    anyhow::bail!("directory '{}' does not exist", parent.display());
                }
                anyhow::bail!(
                    "mdvs.toml not found in '{}' — run 'mdvs init {}' to initialize",
                    parent.display(),
                    parent.display()
                );
            }
            Err(e) => anyhow::bail!("failed to read {}: {e}", path.display()),
        };
        let config: MdvsToml = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("failed to parse {}: {e}", path.display()))?;
        Ok(config)
    }

    /// Validate config invariants that can be broken by manual edits.
    ///
    /// Eight invariants are checked:
    /// 1. A field cannot appear in both `[fields].ignore` and `[[fields.field]]`.
    /// 2. All globs in `allowed` and `required` must end with `/*` or `/**`, or be
    ///    exactly `*` or `**`.
    /// 3. Every required glob must be covered by some allowed glob.
    /// 4. Constraints are valid for the field's type (type applicability,
    ///    well-formed values, pairwise compatibility).
    /// 5. Each preprocess entry must be applicable to the field type, and the
    ///    list must contain no duplicates.
    /// 6. A `[[fields.field]]` cannot use top-level `Object` as its type — use
    ///    dotted-name leaf fields instead (per TODO-0097). Object remains
    ///    valid as `Array`'s inner type.
    /// 7. Field names must not start or end with `.`, nor contain empty
    ///    segments (`..`). Names without dots are unaffected.
    /// 8. No shape conflicts: a name cannot be declared both as a leaf and
    ///    as a parent of nested leaves (e.g., `foo` *and* `foo.bar`).
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
            // Invariant 7: dotted field name well-formedness
            if let Err(msg) = validate_field_name(&field.name) {
                anyhow::bail!("{msg}");
            }

            // Invariant 6: top-level Object is not a valid field type.
            // Express nested structure via dotted-name leaves (Wave C).
            if let Ok(FieldType::Object(_)) = FieldType::try_from(&field.field_type) {
                anyhow::bail!(
                    "field '{}': top-level Object type is not supported — flatten into dotted-name leaf fields (e.g. '{}.<child>').",
                    field.name,
                    field.name
                );
            }

            // Invariant 9 (TODO-0155): Object inside Array is not representable
            // on disk. Defense in depth alongside the parser rejection — catches
            // --from-jsonschema imports and programmatic construction.
            if let Ok(ft) = FieldType::try_from(&field.field_type)
                && type_contains_object_inside_array(&ft)
            {
                anyhow::bail!(
                    "field '{}': Array(Object{{...}}) is not representable on disk. \
                     Consider parallel scalar arrays (see TODO-0156).",
                    field.name
                );
            }

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

            // Invariant 4: constraints are valid for field type
            if let Some(ref constraints) = field.constraints {
                match FieldType::try_from(&field.field_type) {
                    Ok(ft) => {
                        let errors = constraints.validate_config(&field.name, &ft);
                        if let Some(first) = errors.into_iter().next() {
                            anyhow::bail!("{first}");
                        }
                    }
                    Err(e) => {
                        anyhow::bail!("field '{}': invalid type — {e}", field.name);
                    }
                }
            }

            // Invariant 5: preprocess entries must be applicable to the field
            //              type and must not duplicate.
            if !field.preprocess.is_empty() {
                let ft = FieldType::try_from(&field.field_type)
                    .map_err(|e| anyhow::anyhow!("field '{}': invalid type — {e}", field.name))?;
                let mut seen: std::collections::HashSet<ValueStage> =
                    std::collections::HashSet::new();
                for stage in &field.preprocess {
                    if !seen.insert(*stage) {
                        anyhow::bail!("field '{}': duplicate preprocess '{}'", field.name, stage);
                    }
                    if !stage.applies_to(&ft) {
                        anyhow::bail!(
                            "field '{}': preprocess '{}' is not applicable to type {} (applies only to {})",
                            field.name,
                            stage,
                            field.field_type,
                            stage.applicable_types(),
                        );
                    }
                }
            }
        }

        // Invariant 8: no shape conflicts — a name cannot be declared both
        // as a leaf and as a parent of nested leaves.
        //
        // For each field, every strict prefix of its dotted name is a
        // "parent path" (in the tree sense). The same name must not also
        // appear as a leaf in some other entry's `name`.
        let leaves: std::collections::HashSet<&str> =
            self.fields.field.iter().map(|f| f.name.as_str()).collect();
        for field in &self.fields.field {
            let segments: Vec<&str> = field.name.split('.').collect();
            for i in 1..segments.len() {
                let prefix = segments[..i].join(".");
                if leaves.contains(prefix.as_str()) {
                    anyhow::bail!(
                        "shape conflict: '{}' is declared both as a leaf and as a parent of nested leaves (e.g., '{}'). Pick one — keep '{}' as a leaf, or remove it and keep only the nested fields.",
                        prefix,
                        field.name,
                        prefix
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

/// Check a field name for dot-separator well-formedness.
///
/// Field names may contain `.` to indicate nested-leaf membership
/// (`calibration.baseline.wavelength` — see TODO-0097). The dot is purely a
/// separator: each segment between dots must be non-empty, and the name
/// itself must not start or end with `.`.
///
/// Segments themselves may contain spaces or punctuation (existing behavior:
/// `lab section` is a valid name today and must stay valid).
fn validate_field_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("field name cannot be empty".into());
    }
    if name.starts_with('.') {
        return Err(format!("field name '{name}' cannot start with '.'"));
    }
    if name.ends_with('.') {
        return Err(format!("field name '{name}' cannot end with '.'"));
    }
    if name.contains("..") {
        return Err(format!(
            "field name '{name}' contains an empty segment between dots ('..')"
        ));
    }
    Ok(())
}

/// True iff a `FieldType` tree contains an `Object` directly inside an
/// `Array` (transitively). Used by invariant 9 (TODO-0155).
///
/// Top-level `Object` is rejected by invariant 6 separately; this helper
/// only walks past `Array` and `Object` nodes looking for `Array(Object{...})`
/// or deeper nestings of the same pattern.
fn type_contains_object_inside_array(ft: &FieldType) -> bool {
    match ft {
        FieldType::Array(inner) => {
            matches!(inner.as_ref(), FieldType::Object(_))
                || type_contains_object_inside_array(inner)
        }
        FieldType::Object(fields) => fields.values().any(type_contains_object_inside_array),
        _ => false,
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

/// Post-process serialized TOML to enforce canonical key order within
/// each `[[fields.field]]` block: `name, type, allowed, required,
/// nullable, constraints, preprocess`.
///
/// `type` is a string after TODO-0155 (e.g. `"Array(String)"`), so the
/// previous sub-table-to-inline-table conversion is no longer needed.
/// Key ordering is still done here because TOML emits keys in struct
/// declaration order at the top level, but post-Wave-C sub-sections
/// (`constraints`, `preprocess`) can still alter the layout depending
/// on whether they're populated.
fn inline_field_types(toml_str: &str) -> anyhow::Result<String> {
    let mut doc = toml_str.parse::<toml_edit::DocumentMut>()?;

    if let Some(fields) = doc.get_mut("fields")
        && let Some(field_array) = fields.get_mut("field")
        && let Some(array_of_tables) = field_array.as_array_of_tables_mut()
    {
        for table in array_of_tables.iter_mut() {
            reorder_field_keys(table);
        }
    }

    Ok(doc.to_string())
}

/// Reorder a `[[fields.field]]` table's keys so they render in the
/// `TomlField` struct declaration order: `name`, `type`, `allowed`,
/// `required`, `nullable`, `constraints`, `preprocess`. Any key not in
/// the canonical order is sorted after the known ones (alphabetically).
fn reorder_field_keys(table: &mut toml_edit::Table) {
    const ORDER: &[&str] = &[
        "name",
        "type",
        "allowed",
        "required",
        "nullable",
        "constraints",
        "preprocess",
    ];
    table.sort_values_by(|k1, _, k2, _| {
        let p1 = ORDER
            .iter()
            .position(|x| *x == k1.get())
            .unwrap_or(ORDER.len());
        let p2 = ORDER
            .iter()
            .position(|x| *x == k2.get())
            .unwrap_or(ORDER.len());
        p1.cmp(&p2).then_with(|| k1.get().cmp(k2.get()))
    });
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
                max_categories: 10,
                min_category_repetition: 3,
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
                        constraints: None,
                        preprocess: vec![],
                    },
                    TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into(), "notes/**".into()],
                        required: vec!["blog/drafts/**".into(), "notes/**".into()],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                    TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                ],
                max_categories: 10,
                min_category_repetition: 3,
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
type = "Array(String)"
allowed = ["blog/**"]
required = []

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
        assert_eq!(parsed.fields.field.len(), 2);

        let title_ft = FieldType::try_from(&parsed.fields.field[0].field_type).unwrap();
        assert_eq!(title_ft, FieldType::String);

        let tags_ft = FieldType::try_from(&parsed.fields.field[1].field_type).unwrap();
        assert_eq!(tags_ft, FieldType::Array(Box::new(FieldType::String)));
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
                    distinct_values: vec![],
                    occurrence_count: 0,
                    preprocess: vec![],
                },
                InferredField {
                    name: "tags".into(),
                    field_type: FieldType::Array(Box::new(FieldType::String)),
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["blog/**".into(), "notes/**".into()],
                    required: vec!["notes/**".into()],
                    nullable: false,
                    distinct_values: vec![],
                    occurrence_count: 0,
                    preprocess: vec![],
                },
                InferredField {
                    name: "title".into(),
                    field_type: FieldType::String,
                    files: vec![PathBuf::from("blog/a.md"), PathBuf::from("notes/b.md")],
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                    nullable: false,
                    distinct_values: vec![],
                    occurrence_count: 0,
                    preprocess: vec![],
                },
            ],
            dropped: vec![],
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
        let schema = InferredSchema {
            fields: vec![],
            dropped: vec![],
        };
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
        let schema = InferredSchema {
            fields: vec![],
            dropped: vec![],
        };
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
                distinct_values: vec![],
                occurrence_count: 0,
                preprocess: vec![],
            }],
            dropped: vec![],
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
    fn write_uses_function_style_string_for_type() {
        let mut doc = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");
        doc.write(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();

        // TODO-0155: function-style strings, not inline tables or section tables.
        assert!(
            content.contains(r#"type = "Array(String)""#),
            "expected function-style array type, got:\n{content}"
        );
        assert!(
            !content.contains("[fields.field.type]"),
            "should not have expanded type sections"
        );
        assert!(
            !content.contains("type = {"),
            "should not emit inline-table type form"
        );

        // Still roundtrips correctly.
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
                max_categories: 10,
                min_category_repetition: 3,
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
type = "Array(String)"
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
                constraints: None,
                preprocess: vec![],
            },
            TomlField {
                name: "b".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["blog/**".into()],
                required: vec!["blog/**".into()],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
            TomlField {
                name: "c".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["people/*".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
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
            constraints: None,
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    // --- Invariant 5: preprocess applicability + duplicates ---

    #[test]
    fn validate_rejects_widen_int_to_float_on_string_field() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![ValueStage::WidenIntToFloat],
        }]);
        let err = config.validate().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("widen_int_to_float"), "got: {msg}");
        assert!(msg.contains("not applicable"), "got: {msg}");
        assert!(msg.contains("Float, Array(Float)"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_coerce_to_string_on_integer_field() {
        let config = full_toml(vec![TomlField {
            name: "count".into(),
            field_type: FieldTypeSerde::Scalar("Integer".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![ValueStage::CoerceToString],
        }]);
        let err = config.validate().unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("coerce_to_string"), "got: {msg}");
        assert!(msg.contains("not applicable"), "got: {msg}");
    }

    #[test]
    fn validate_rejects_duplicate_preprocess() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![ValueStage::CoerceToString, ValueStage::CoerceToString],
        }]);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("duplicate preprocess"));
    }

    #[test]
    fn validate_accepts_applicable_preprocess() {
        let config = full_toml(vec![
            TomlField {
                name: "title".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![ValueStage::CoerceToString],
            },
            TomlField {
                name: "score".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![ValueStage::WidenIntToFloat],
            },
        ]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_array_string_with_coerce_to_string() {
        let config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![ValueStage::CoerceToString],
        }]);
        assert!(config.validate().is_ok());
    }

    // --- Invariant 6: top-level Object rejected; Array(Object) allowed ---

    #[test]
    fn validate_rejects_top_level_object_field() {
        let config = full_toml(vec![TomlField {
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
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("top-level Object"), "got: {err}");
        assert!(err.contains("flatten"), "got: {err}");
        assert!(err.contains("meta.<child>"), "got: {err}");
    }

    #[test]
    fn validate_rejects_array_of_object() {
        // TODO-0155 invariant 9: Array(Object{...}) is rejected at config load.
        let config = full_toml(vec![TomlField {
            name: "readings".into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Object {
                    object: BTreeMap::from([
                        ("time".into(), FieldTypeSerde::Scalar("String".into())),
                        ("value".into(), FieldTypeSerde::Scalar("Float".into())),
                    ]),
                }),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("readings"), "got: {err}");
        assert!(err.contains("Array(Object"), "got: {err}");
        assert!(err.contains("TODO-0156"), "got: {err}");
    }

    #[test]
    fn validate_accepts_array_of_scalar() {
        let config = full_toml(vec![TomlField {
            name: "tags".into(),
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    // --- Invariant 7: dotted field name well-formedness ---

    #[test]
    fn validate_accepts_plain_name() {
        let config = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_dotted_name() {
        let config = full_toml(vec![TomlField {
            name: "calibration.baseline.wavelength".into(),
            field_type: FieldTypeSerde::Scalar("Float".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_accepts_name_with_whitespace() {
        // Existing example_kb behavior: `lab section` is a valid field name.
        let config = full_toml(vec![TomlField {
            name: "lab section".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_leading_dot() {
        let config = full_toml(vec![TomlField {
            name: ".foo".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("cannot start with '.'"), "got: {err}");
    }

    #[test]
    fn validate_rejects_trailing_dot() {
        let config = full_toml(vec![TomlField {
            name: "foo.".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("cannot end with '.'"), "got: {err}");
    }

    #[test]
    fn validate_rejects_empty_segment() {
        let config = full_toml(vec![TomlField {
            name: "foo..bar".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("empty segment"), "got: {err}");
    }

    #[test]
    fn validate_rejects_empty_name() {
        let config = full_toml(vec![TomlField {
            name: "".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("empty"), "got: {err}");
    }

    // --- Invariant 8: shape-conflict among field names ---

    #[test]
    fn validate_rejects_leaf_and_parent_same_name() {
        // `meta` declared both as a leaf AND as a parent (`meta.author`).
        let config = full_toml(vec![
            TomlField {
                name: "meta".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
            TomlField {
                name: "meta.author".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
        ]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("shape conflict"), "got: {err}");
        assert!(err.contains("'meta'"), "got: {err}");
    }

    #[test]
    fn validate_rejects_deep_shape_conflict() {
        // `cal.baseline` declared as a leaf AND as a parent (`cal.baseline.wave`).
        let config = full_toml(vec![
            TomlField {
                name: "cal.baseline".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
            TomlField {
                name: "cal.baseline.wave".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
        ]);
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("shape conflict"), "got: {err}");
        assert!(err.contains("'cal.baseline'"), "got: {err}");
    }

    #[test]
    fn validate_accepts_siblings_under_shared_prefix() {
        // `cal.x` and `cal.y` share the `cal` intermediate but neither
        // declares `cal` as a leaf — no conflict.
        let config = full_toml(vec![
            TomlField {
                name: "cal.x".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
            TomlField {
                name: "cal.y".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            },
        ]);
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

    // --- constraints integration tests ---

    #[test]
    fn toml_field_with_constraints_roundtrip() {
        let doc = full_toml(vec![TomlField {
            name: "status".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![
                    toml::Value::String("draft".into()),
                    toml::Value::String("published".into()),
                ]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        let toml_str = toml::to_string(&doc).unwrap();
        let parsed: MdvsToml = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn parse_handwritten_constraints() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]

[[fields.field]]
name = "status"
type = "String"

[fields.field.constraints]
categories = ["draft", "published", "archived"]
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        let f = &parsed.fields.field[0];
        assert_eq!(f.name, "status");
        let cats = f.constraints.as_ref().unwrap().categories.as_ref().unwrap();
        assert_eq!(cats.len(), 3);
    }

    #[test]
    fn absent_constraints_parses_to_none() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]

[[fields.field]]
name = "title"
type = "String"
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        assert!(parsed.fields.field[0].constraints.is_none());
    }

    #[test]
    fn none_constraints_not_serialized() {
        let doc = full_toml(vec![TomlField {
            name: "title".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }]);
        let toml_str = toml::to_string(&doc).unwrap();
        assert!(!toml_str.contains("constraints"));
    }

    #[test]
    fn validate_rejects_categories_on_boolean() {
        let config = full_toml(vec![TomlField {
            name: "draft".into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![toml::Value::String("yes".into())]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("does not apply"));
    }

    #[test]
    fn validate_accepts_valid_string_categories() {
        let config = full_toml(vec![TomlField {
            name: "status".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![
                    toml::Value::String("draft".into()),
                    toml::Value::String("published".into()),
                ]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_mismatched_category_values() {
        let config = full_toml(vec![TomlField {
            name: "status".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![toml::Value::Integer(1)]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("does not match field type"));
    }

    #[test]
    fn validate_rejects_invalid_type_with_constraints() {
        let config = full_toml(vec![TomlField {
            name: "status".into(),
            field_type: FieldTypeSerde::Scalar("Strng".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![toml::Value::String("a".into())]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("invalid type"));
    }

    #[test]
    fn threshold_fields_roundtrip() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]
max_categories = 15
min_category_repetition = 3
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.fields.max_categories, 15);
        assert_eq!(parsed.fields.min_category_repetition, 3);
    }

    #[test]
    fn threshold_fields_default_when_absent() {
        let toml_str = r#"
[scan]
glob = "**"
include_bare_files = false

[fields]
"#;
        let parsed: MdvsToml = toml::from_str(toml_str).unwrap();
        assert_eq!(parsed.fields.max_categories, 10);
        assert_eq!(parsed.fields.min_category_repetition, 3);
    }

    #[test]
    fn default_thresholds_not_serialized() {
        let doc = full_toml(vec![]);
        let toml_str = toml::to_string(&doc).unwrap();
        assert!(!toml_str.contains("max_categories"));
        assert!(!toml_str.contains("min_category_repetition"));
    }

    #[test]
    fn write_preserves_constraints_subtable() {
        let mut doc = full_toml(vec![TomlField {
            name: "status".into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(vec![
                    toml::Value::String("draft".into()),
                    toml::Value::String("published".into()),
                ]),
                ..Default::default()
            }),
            preprocess: vec![],
        }]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mdvs.toml");
        doc.write(&path).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[fields.field.constraints]"));
        assert!(content.contains("categories"));
        let loaded = MdvsToml::read(&path).unwrap();
        assert_eq!(loaded, doc);
    }

    #[test]
    fn read_missing_directory_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent_dir").join("mdvs.toml");
        let err = MdvsToml::read(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("does not exist"), "got: {msg}");
        assert!(msg.contains("nonexistent_dir"), "got: {msg}");
    }

    #[test]
    fn read_missing_toml_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mdvs.toml");
        let err = MdvsToml::read(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("mdvs.toml not found"), "got: {msg}");
        assert!(msg.contains("mdvs init"), "got: {msg}");
    }

    #[test]
    fn read_invalid_toml_error() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("mdvs.toml");
        fs::write(&path, "this is not valid toml = [unclosed").unwrap();
        let err = MdvsToml::read(&path).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("failed to parse"), "got: {msg}");
    }
}
