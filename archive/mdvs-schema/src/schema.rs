use std::collections::HashSet;
use std::path::Path;

use serde::{Deserialize, Serialize};

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

/// Which frontmatter delimiter formats to recognize.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FrontmatterFormat {
    /// Recognize both YAML (`---`) and TOML (`+++`) delimiters.
    #[default]
    Both,
    /// Only recognize YAML (`---`) delimiters.
    Yaml,
    /// Only recognize TOML (`+++`) delimiters.
    Toml,
}

impl std::fmt::Display for FrontmatterFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrontmatterFormat::Both => write!(f, "both"),
            FrontmatterFormat::Yaml => write!(f, "yaml"),
            FrontmatterFormat::Toml => write!(f, "toml"),
        }
    }
}

impl std::str::FromStr for FrontmatterFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "both" => Ok(FrontmatterFormat::Both),
            "yaml" => Ok(FrontmatterFormat::Yaml),
            "toml" => Ok(FrontmatterFormat::Toml),
            _ => Err(format!("unknown frontmatter format '{s}', expected 'both', 'yaml', or 'toml'")),
        }
    }
}

impl serde::Serialize for FrontmatterFormat {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for FrontmatterFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
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
    glob: Option<String>,
    include_bare_files: Option<bool>,
    frontmatter_format: Option<FrontmatterFormat>,
}

#[derive(Debug, Deserialize)]
struct RawFieldsSection {
    #[serde(default)]
    field: Vec<RawFieldDef>,
}

/// A parsed and validated schema.
#[derive(Debug)]
pub struct Schema {
    /// Glob pattern for matching markdown files.
    pub glob: String,
    /// Whether to include files without frontmatter in analysis.
    pub include_bare_files: bool,
    /// Which frontmatter formats to recognize.
    pub frontmatter_format: FrontmatterFormat,
    /// Field definitions loaded from the TOML config.
    pub fields: Vec<FieldDef>,
}

impl Schema {
    /// Load a schema from a file path.
    pub fn from_file(path: &Path) -> Result<Self, SchemaError> {
        let content = std::fs::read_to_string(path)?;
        content.parse()
    }

    /// Return field definitions that are allowed at a given relative file path.
    pub fn rules_for_path(&self, rel_path: &str) -> Vec<&FieldDef> {
        self.fields
            .iter()
            .filter(|f| f.is_allowed_at(rel_path))
            .collect()
    }

    /// Generate TOML string representation of this schema.
    pub fn to_toml_string(&self) -> String {
        let ser = SerSchema {
            directory: SerDirectory {
                glob: &self.glob,
                include_bare_files: self.include_bare_files,
                frontmatter_format: &self.frontmatter_format,
            },
            fields: SerFields {
                field: self.fields.iter().map(SerFieldDef::from).collect(),
            },
        };
        toml::to_string_pretty(&ser).expect("schema serialization should not fail")
    }
}

#[derive(Serialize)]
struct SerSchema<'a> {
    directory: SerDirectory<'a>,
    fields: SerFields<'a>,
}

#[derive(Serialize)]
struct SerDirectory<'a> {
    glob: &'a str,
    include_bare_files: bool,
    frontmatter_format: &'a FrontmatterFormat,
}

#[derive(Serialize)]
struct SerFields<'a> {
    field: Vec<SerFieldDef<'a>>,
}

#[derive(Serialize)]
struct SerFieldDef<'a> {
    name: &'a str,
    #[serde(rename = "type")]
    field_type: &'a FieldType,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    allowed: &'a Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    required: &'a Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: &'a Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    values: &'a Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    date_format: &'a Option<String>,
}

impl<'a> From<&'a FieldDef> for SerFieldDef<'a> {
    fn from(f: &'a FieldDef) -> Self {
        SerFieldDef {
            name: &f.name,
            field_type: &f.field_type,
            allowed: &f.allowed,
            required: &f.required,
            pattern: &f.pattern,
            values: &f.values,
            date_format: &f.date_format,
        }
    }
}

impl std::str::FromStr for Schema {
    type Err = SchemaError;

    fn from_str(s: &str) -> Result<Self, SchemaError> {
        let raw: RawSchema = toml::from_str(s)?;

        let (glob, include_bare_files, frontmatter_format) = match raw.directory {
            Some(d) => (
                d.glob.unwrap_or_else(|| "**".to_string()),
                d.include_bare_files.unwrap_or(false),
                d.frontmatter_format.unwrap_or_default(),
            ),
            None => ("**".to_string(), false, FrontmatterFormat::default()),
        };

        let raw_fields = match raw.fields {
            Some(section) => section.field,
            None => Vec::new(),
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
            include_bare_files,
            frontmatter_format,
            fields,
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

    // date_format only valid for date
    if def.date_format.is_some() && def.field_type != FieldType::Date {
        return Err(SchemaError::Validation(format!(
            "field '{}': 'date_format' only valid for date type",
            def.name
        )));
    }

    // required ⊆ allowed: can't require a field that's not allowed anywhere
    if !def.required.is_empty() && def.allowed.is_empty() {
        return Err(SchemaError::Validation(format!(
            "field '{}': has required patterns but allowed is empty (required ⊆ allowed)",
            def.name
        )));
    }

    // validate allowed glob patterns
    for pattern in &def.allowed {
        globset::Glob::new(pattern).map_err(|e| {
            SchemaError::Validation(format!(
                "field '{}': invalid allowed glob '{}': {}",
                def.name, pattern, e
            ))
        })?;
    }

    // validate required glob patterns
    for pattern in &def.required {
        globset::Glob::new(pattern).map_err(|e| {
            SchemaError::Validation(format!(
                "field '{}': invalid required glob '{}': {}",
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
glob = "**"

[[fields.field]]
name = "title"
type = "string"

[[fields.field]]
name = "tags"
type = "string[]"

[[fields.field]]
name = "date"
type = "date"
required = ["**"]
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.glob, "**");
        assert_eq!(schema.fields.len(), 3);

        let date = schema.fields.iter().find(|f| f.name == "date").unwrap();
        assert!(date.is_required_at("any/path.md"));
        assert_eq!(date.field_type, FieldType::Date);
    }

    #[test]
    fn parse_enum_field() {
        let toml = r#"
[[fields.field]]
name = "status"
type = "enum"
values = ["draft", "review", "published"]
required = ["blog/**"]
allowed = ["blog/**"]
"#;
        let schema = toml.parse::<Schema>().unwrap();
        let status = &schema.fields[0];
        assert_eq!(status.field_type, FieldType::Enum);
        assert_eq!(status.values, vec!["draft", "review", "published"]);
        assert!(status.is_required_at("blog/post.md"));
        assert!(!status.is_required_at("notes/idea.md"));
        assert_eq!(status.allowed, vec!["blog/**"]);
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
allowed = ["papers/**"]
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
required = ["**"]

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

    #[test]
    fn default_allowed_is_everywhere() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.fields[0].allowed, vec!["**"]);
        assert!(schema.fields[0].required.is_empty());
    }

    #[test]
    fn required_without_allowed_fails() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"
allowed = []
required = ["**"]
"#;
        let err = toml.parse::<Schema>().unwrap_err();
        assert!(err.to_string().contains("required ⊆ allowed"));
    }

    #[test]
    fn frontmatter_format_defaults_to_both() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.frontmatter_format, FrontmatterFormat::Both);
    }

    #[test]
    fn frontmatter_format_parsed_from_toml() {
        let toml = r#"
[directory]
frontmatter_format = "yaml"

[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.frontmatter_format, FrontmatterFormat::Yaml);
    }

    #[test]
    fn frontmatter_format_roundtrip() {
        let toml = r#"
[directory]
frontmatter_format = "toml"

[[fields.field]]
name = "title"
type = "string"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.frontmatter_format, FrontmatterFormat::Toml);

        let output = schema.to_toml_string();
        let schema2 = output.parse::<Schema>().unwrap();
        assert_eq!(schema2.frontmatter_format, FrontmatterFormat::Toml);
    }

    #[test]
    fn parse_date_format() {
        let toml = r#"
[[fields.field]]
name = "published"
type = "date"
date_format = "%d/%m/%Y"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(
            schema.fields[0].date_format,
            Some("%d/%m/%Y".to_string())
        );
    }

    #[test]
    fn date_format_on_non_date_fails() {
        let toml = r#"
[[fields.field]]
name = "title"
type = "string"
date_format = "%Y-%m-%d"
"#;
        let err = toml.parse::<Schema>().unwrap_err();
        assert!(err.to_string().contains("'date_format' only valid for date type"));
    }

    #[test]
    fn date_format_roundtrip() {
        let toml = r#"
[[fields.field]]
name = "created"
type = "date"
date_format = "%d/%m/%Y"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        let output = schema.to_toml_string();
        let schema2 = output.parse::<Schema>().unwrap();
        assert_eq!(
            schema2.fields[0].date_format,
            Some("%d/%m/%Y".to_string())
        );
    }

    #[test]
    fn no_date_format_default() {
        let toml = r#"
[[fields.field]]
name = "date"
type = "date"
"#;
        let schema = toml.parse::<Schema>().unwrap();
        assert_eq!(schema.fields[0].date_format, None);
    }
}
