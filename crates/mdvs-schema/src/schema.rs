use std::collections::HashSet;
use std::path::Path;

use globset::{Glob, GlobMatcher};
use serde::Deserialize;

use crate::FieldType;
use crate::field_def::{FieldDef, RawFieldDef};

/// Errors that can occur when loading or using a schema.
#[derive(Debug)]
pub enum SchemaError {
    /// File I/O error.
    Io(std::io::Error),
    /// TOML deserialization error.
    Parse(toml::de::Error),
    /// Semantic validation error (e.g. duplicate field names, enum without values).
    Validation(String),
    /// Invalid glob pattern.
    Glob(globset::Error),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaError::Io(e) => write!(f, "IO error: {e}"),
            SchemaError::Parse(e) => write!(f, "TOML parse error: {e}"),
            SchemaError::Validation(msg) => write!(f, "schema validation error: {msg}"),
            SchemaError::Glob(e) => write!(f, "glob error: {e}"),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<std::io::Error> for SchemaError {
    fn from(e: std::io::Error) -> Self {
        SchemaError::Io(e)
    }
}

impl From<toml::de::Error> for SchemaError {
    fn from(e: toml::de::Error) -> Self {
        SchemaError::Parse(e)
    }
}

impl From<globset::Error> for SchemaError {
    fn from(e: globset::Error) -> Self {
        SchemaError::Glob(e)
    }
}

/// Raw TOML structure for deserialization.
#[derive(Debug, Deserialize)]
struct RawSchema {
    directory: Option<DirectoryConfig>,
    fields: Option<RawFieldsSection>,
}

#[derive(Debug, Deserialize)]
struct DirectoryConfig {
    #[allow(dead_code)]
    glob: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawFieldsSection {
    promote_threshold: Option<f64>,
    #[serde(default)]
    field: Vec<RawFieldDef>,
}

/// A parsed and validated schema.
#[derive(Debug)]
pub struct Schema {
    /// Glob pattern for matching markdown files.
    pub glob: String,
    /// Field definitions loaded from the TOML config.
    pub fields: Vec<FieldDef>,
    /// Auto-promote threshold from the `[fields]` section.
    pub promote_threshold: Option<f64>,
}

impl Schema {
    /// Load a schema from a file path.
    pub fn from_file(path: &Path) -> Result<Self, SchemaError> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// Return field definitions that apply to a given relative file path.
    pub fn rules_for_path(&self, rel_path: &str) -> Vec<&FieldDef> {
        self.fields
            .iter()
            .filter(|f| {
                if f.paths.is_empty() {
                    return true;
                }
                f.paths.iter().any(|pattern| {
                    Glob::new(pattern)
                        .ok()
                        .map(|g| g.compile_matcher())
                        .is_some_and(|m: GlobMatcher| m.is_match(rel_path))
                })
            })
            .collect()
    }

    /// Generate TOML string representation of this schema.
    pub fn to_toml_string(&self) -> String {
        let mut out = String::new();

        out.push_str("[directory]\n");
        out.push_str(&format!("glob = {:?}\n", self.glob));
        out.push('\n');

        out.push_str("[fields]\n");
        if let Some(threshold) = self.promote_threshold {
            out.push_str(&format!("promote_threshold = {threshold}\n"));
        }
        out.push('\n');

        for field in &self.fields {
            out.push_str("[[fields.field]]\n");
            out.push_str(&format!("name = {:?}\n", field.name));
            out.push_str(&format!("type = {:?}\n", field.field_type.to_string()));

            if field.required {
                out.push_str("required = true\n");
            }

            if !field.paths.is_empty() {
                let paths: Vec<String> = field.paths.iter().map(|p| format!("{p:?}")).collect();
                out.push_str(&format!("paths = [{}]\n", paths.join(", ")));
            }

            if let Some(pattern) = &field.pattern {
                out.push_str(&format!("pattern = {:?}\n", pattern));
            }

            if !field.values.is_empty() {
                let vals: Vec<String> = field.values.iter().map(|v| format!("{v:?}")).collect();
                out.push_str(&format!("values = [{}]\n", vals.join(", ")));
            }

            if field.promoted {
                out.push_str("promoted = true\n");
            }

            out.push('\n');
        }

        out
    }
}

impl std::str::FromStr for Schema {
    type Err = SchemaError;

    fn from_str(s: &str) -> Result<Self, SchemaError> {
        let raw: RawSchema = toml::from_str(s)?;

        let glob = raw
            .directory
            .and_then(|d| d.glob)
            .unwrap_or_else(|| "**/*.md".to_string());

        let (promote_threshold, raw_fields) = match raw.fields {
            Some(section) => (section.promote_threshold, section.field),
            None => (None, Vec::new()),
        };

        // Check for duplicate field names
        let mut seen = HashSet::new();
        for raw_def in &raw_fields {
            if !seen.insert(&raw_def.name) {
                return Err(SchemaError::Validation(format!(
                    "duplicate field name '{}'",
                    raw_def.name
                )));
            }
        }

        let mut fields: Vec<FieldDef> = raw_fields
            .into_iter()
            .map(|raw_def| {
                let def = raw_def.into_field_def();
                validate_field_def(&def)?;
                Ok(def)
            })
            .collect::<Result<Vec<_>, SchemaError>>()?;

        fields.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(Schema {
            glob,
            fields,
            promote_threshold,
        })
    }
}

fn validate_field_def(def: &FieldDef) -> Result<(), SchemaError> {
    // enum fields must have values
    if def.field_type == FieldType::Enum && def.values.is_empty() {
        return Err(SchemaError::Validation(format!(
            "field '{}': enum type requires 'values' list",
            def.name
        )));
    }

    // pattern only valid for string/date
    if def.pattern.is_some() && !matches!(def.field_type, FieldType::String | FieldType::Date) {
        return Err(SchemaError::Validation(format!(
            "field '{}': 'pattern' only valid for string or date types",
            def.name
        )));
    }

    // validate glob patterns
    for pattern in &def.paths {
        Glob::new(pattern).map_err(|e| {
            SchemaError::Validation(format!(
                "field '{}': invalid glob '{}': {}",
                def.name, pattern, e
            ))
        })?;
    }

    // validate regex pattern
    if let Some(pat) = &def.pattern {
        regex::Regex::new(pat).map_err(|e| {
            SchemaError::Validation(format!(
                "field '{}': invalid regex '{}': {}",
                def.name, pat, e
            ))
        })?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_schema() {
        let toml = r#"
[directory]
glob = "**/*.md"

[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "tags"
type = "string[]"

[[fields.field]]
name = "date"
type = "date"
required = true
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.glob, "**/*.md");
        assert_eq!(schema.fields.len(), 3);

        let date = schema.fields.iter().find(|f| f.name == "date").unwrap();
        assert!(date.required);
        assert_eq!(date.field_type, FieldType::Date);
    }

    #[test]
    fn parse_enum_field() {
        let toml = r#"
[[fields.field]]
name = "status"
type = "enum"
values = ["draft", "review", "published"]
required = true
paths = ["blog/**"]
"#;
        let schema = toml.parse::<Schema>().unwrap();
        let status = &schema.fields[0];
        assert_eq!(status.field_type, FieldType::Enum);
        assert_eq!(status.values, vec!["draft", "review", "published"]);
        assert!(status.required);
        assert_eq!(status.paths, vec!["blog/**"]);
    }

    #[test]
    fn enum_without_values_fails() {
        let toml = r#"
[[fields.field]]
name = "status"
type = "enum"
"#;
        let err = toml.parse::<Schema>().unwrap_err();
        assert!(err.to_string().contains("enum type requires 'values' list"));
    }

    #[test]
    fn rules_for_path_filters() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "doi"
type = "string"
paths = ["papers/**"]
"#;
        let schema = toml.parse::<Schema>().unwrap();

        let rules = schema.rules_for_path("blog/post.md");
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "title");

        let rules = schema.rules_for_path("papers/paper1.md");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn default_type_is_string() {
        let toml = r#"
[[fields.field]]
name = "title"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.fields[0].field_type, FieldType::String);
    }

    #[test]
    fn roundtrip_toml() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"
required = true

[[fields.field]]
name = "tags"
type = "string[]"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        let output = schema.to_toml_string();

        // Should be re-parseable
        let schema2 = output.parse::<Schema>().unwrap();
        assert_eq!(schema2.fields.len(), 2);
    }

    #[test]
    fn duplicate_field_names_fails() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "title"
type = "integer"
"#;
        let err = toml.parse::<Schema>().unwrap_err();
        assert!(err.to_string().contains("duplicate field name 'title'"));
    }

    #[test]
    fn promote_threshold_parsed() {
        let toml = r#"
[fields]
promote_threshold = 0.75

[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.promote_threshold, Some(0.75));
        assert_eq!(schema.fields.len(), 1);
    }

    #[test]
    fn unknown_sections_ignored() {
        let toml = r#"
[model]
name = "some-model"
dimensions = 256

[search]
top_k = 10

[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "title");
    }
}
