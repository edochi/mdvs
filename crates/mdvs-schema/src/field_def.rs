use globset::Glob;
use serde::Deserialize;

use crate::FieldType;

/// A validated field definition from the TOML config.
#[derive(Debug, Clone)]
pub struct FieldDef {
    /// Field name as it appears in frontmatter.
    pub name: String,
    /// Expected value type.
    pub field_type: FieldType,
    /// Glob patterns where this field may appear.
    /// `[]` = nowhere, `["**"]` = everywhere.
    pub allowed: Vec<String>,
    /// Glob patterns where this field must be present.
    /// `[]` = not required anywhere, `["**"]` = required everywhere.
    pub required: Vec<String>,
    /// Regex pattern the value must match (string/date fields).
    pub pattern: Option<String>,
    /// Allowed values (enum fields).
    pub values: Vec<String>,
    /// Expected date format in chrono strftime syntax (date fields only).
    pub date_format: Option<String>,
}

impl FieldDef {
    /// Check if this field is allowed at a given relative file path.
    ///
    /// Returns `true` if any `allowed` pattern matches the path.
    /// Returns `false` if `allowed` is empty (field not allowed anywhere).
    pub fn is_allowed_at(&self, path: &str) -> bool {
        matches_any_pattern(&self.allowed, path)
    }

    /// Check if this field is required at a given relative file path.
    ///
    /// Returns `true` if any `required` pattern matches the path.
    /// Returns `false` if `required` is empty (field not required anywhere).
    pub fn is_required_at(&self, path: &str) -> bool {
        matches_any_pattern(&self.required, path)
    }
}

/// Check if a path matches any of the given glob patterns.
fn matches_any_pattern(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|pattern| {
        Glob::new(pattern)
            .ok()
            .map(|g| g.compile_matcher())
            .is_some_and(|m| m.is_match(path))
    })
}

fn default_allowed() -> Vec<String> {
    vec!["**".to_string()]
}

/// Raw serde struct for deserializing a field entry from TOML.
#[derive(Debug, Deserialize)]
pub(crate) struct RawFieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub field_type: Option<FieldType>,
    #[serde(default = "default_allowed")]
    pub allowed: Vec<String>,
    #[serde(default)]
    pub required: Vec<String>,
    pub pattern: Option<String>,
    #[serde(default)]
    pub values: Vec<String>,
    pub date_format: Option<String>,
}

impl RawFieldDef {
    pub(crate) fn into_field_def(self) -> FieldDef {
        let field_type = self.field_type.unwrap_or(FieldType::String);
        FieldDef {
            name: self.name,
            field_type,
            allowed: self.allowed,
            required: self.required,
            pattern: self.pattern,
            values: self.values,
            date_format: self.date_format,
        }
    }
}
