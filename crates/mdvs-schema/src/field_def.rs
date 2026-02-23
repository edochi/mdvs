use serde::Deserialize;

use crate::FieldType;

/// A field definition from `frontmatter.toml`.
#[derive(Debug, Clone)]
pub struct FieldDef {
    pub name: String,
    pub field_type: FieldType,
    pub required: bool,
    /// Glob patterns restricting which paths this rule applies to.
    /// Empty means applies to all files.
    pub paths: Vec<String>,
    /// Regex pattern the value must match (string/date fields).
    pub pattern: Option<String>,
    /// Allowed values (enum fields).
    pub values: Vec<String>,
    /// Whether this field is promoted to a SQL column.
    pub promoted: bool,
}

/// Raw serde struct for deserializing a field entry from TOML.
#[derive(Debug, Deserialize)]
pub(crate) struct RawFieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: Option<FieldType>,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    pub pattern: Option<String>,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub promoted: bool,
}

impl RawFieldDef {
    pub(crate) fn into_field_def(self) -> FieldDef {
        let field_type = self.field_type.unwrap_or(FieldType::String);
        FieldDef {
            name: self.name,
            field_type,
            required: self.required,
            paths: self.paths,
            pattern: self.pattern,
            values: self.values,
            promoted: self.promoted,
        }
    }
}
