use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// Controls whether command output is rendered as plain text or machine-readable JSON.
#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pretty-printed tables and summaries for terminal display.
    Text,
    /// Structured JSON for piping into other tools.
    Json,
}

/// Hint about special characters in a field name that affect `--where` clause usage.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FieldHint {
    /// Field name contains single quotes — escape with `''` in `--where`.
    #[serde(rename = "escape single quotes")]
    EscapeSingleQuotes,
    /// Field name contains double quotes — escape with `""` in `--where`.
    #[serde(rename = "escape double quotes")]
    EscapeDoubleQuotes,
}

impl fmt::Display for FieldHint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldHint::EscapeSingleQuotes => write!(f, "' → '' in --where"),
            FieldHint::EscapeDoubleQuotes => write!(f, "\" → \"\" in --where"),
        }
    }
}

/// Compute hints for a field name based on special characters it contains.
pub fn field_hints(name: &str) -> Vec<FieldHint> {
    let mut hints = Vec::new();
    if name.contains('\'') {
        hints.push(FieldHint::EscapeSingleQuotes);
    }
    if name.contains('"') {
        hints.push(FieldHint::EscapeDoubleQuotes);
    }
    hints
}

/// Format a list of hints as a comma-separated string for table display.
pub fn format_hints(hints: &[FieldHint]) -> String {
    hints
        .iter()
        .map(|h| h.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A frontmatter field discovered during scanning, with its inferred type and prevalence.
#[derive(Debug, Serialize)]
pub struct DiscoveredField {
    /// Field name as it appears in frontmatter YAML keys.
    pub name: String,
    /// Human-readable representation of the inferred type (e.g. `"String"`, `"Integer"`).
    pub field_type: String,
    /// Number of files that contain this field.
    pub files_found: usize,
    /// Total number of scanned files (for computing prevalence).
    pub total_files: usize,
    /// Glob patterns where this field appears (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
    /// Glob patterns where this field is required in every file (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Vec<String>>,
    /// Whether null values are accepted for this field.
    pub nullable: bool,
    /// Hints about special characters in the field name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<FieldHint>,
}

/// A field whose definition changed between the previous and current scan.
#[derive(Debug, Serialize)]
pub struct ChangedField {
    /// Field name.
    pub name: String,
    /// Which aspects of the field definition changed.
    pub changes: Vec<FieldChange>,
}

/// A single aspect of a field definition that changed.
#[derive(Debug, Serialize)]
#[serde(tag = "aspect", rename_all = "snake_case")]
pub enum FieldChange {
    /// The inferred type changed.
    Type {
        /// Previous type.
        old: String,
        /// New type.
        new: String,
    },
    /// The allowed glob patterns changed.
    Allowed {
        /// Previous allowed patterns.
        old: Vec<String>,
        /// New allowed patterns.
        new: Vec<String>,
    },
    /// The required glob patterns changed.
    Required {
        /// Previous required patterns.
        old: Vec<String>,
        /// New required patterns.
        new: Vec<String>,
    },
    /// The nullable flag changed.
    Nullable {
        /// Previous value.
        old: bool,
        /// New value.
        new: bool,
    },
}

impl FieldChange {
    /// Short label for this change aspect.
    pub fn label(&self) -> &'static str {
        match self {
            FieldChange::Type { .. } => "type",
            FieldChange::Allowed { .. } => "allowed",
            FieldChange::Required { .. } => "required",
            FieldChange::Nullable { .. } => "nullable",
        }
    }

    /// Return `(old, new)` strings for verbose table columns.
    pub fn format_old_new(&self) -> (String, String) {
        match self {
            FieldChange::Type { old, new } => (old.clone(), new.clone()),
            FieldChange::Allowed { old, new } => (format_globs(old), format_globs(new)),
            FieldChange::Required { old, new } => (format_globs(old), format_globs(new)),
            FieldChange::Nullable { old, new } => (old.to_string(), new.to_string()),
        }
    }
}

/// Format glob patterns as a bracketed list: `["a", "b"]`.
fn format_globs(globs: &[String]) -> String {
    if globs.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = globs.iter().map(|g| format!("\"{g}\"")).collect();
        format!("[{}]", items.join(", "))
    }
}

/// A field that disappeared from all files during re-inference.
#[derive(Debug, Serialize)]
pub struct RemovedField {
    /// Field name.
    pub name: String,
    /// Previous glob patterns where this field appeared (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allowed: Option<Vec<String>>,
}

/// Category of a frontmatter validation failure.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub enum ViolationKind {
    /// A field marked `required` is absent from the file's frontmatter.
    MissingRequired,
    /// The field's value does not match the expected type.
    WrongType,
    /// The field is not declared in `mdvs.toml` and is not in the ignore list.
    Disallowed,
    /// A non-nullable field has a null value.
    NullNotAllowed,
}

/// A single file that failed a particular field validation rule.
#[derive(Debug, Serialize)]
pub struct ViolatingFile {
    /// Path to the offending markdown file.
    pub path: PathBuf,
    /// Optional context about the violation (e.g. the actual type found).
    pub detail: Option<String>,
}

/// Groups all files that violate a specific validation rule on a single field.
#[derive(Debug, Serialize)]
pub struct FieldViolation {
    /// Name of the frontmatter field.
    pub field: String,
    /// What kind of violation occurred.
    pub kind: ViolationKind,
    /// Human-readable description of the rule that was broken.
    pub rule: String,
    /// Files that triggered this violation.
    pub files: Vec<ViolatingFile>,
}

/// A frontmatter field found during check that is not yet tracked in `mdvs.toml`.
#[derive(Debug, Serialize)]
pub struct NewField {
    /// Field name.
    pub name: String,
    /// Number of files containing this field.
    pub files_found: usize,
    /// Paths of files containing this field (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<PathBuf>>,
}

/// Format a file count with correct pluralization: `"1 file"` / `"3 files"`.
pub fn format_file_count(n: usize) -> String {
    if n == 1 {
        "1 file".to_string()
    } else {
        format!("{n} files")
    }
}

/// Format a byte count as human-readable size: `"256 B"`, `"1.2 KB"`, `"12.4 MB"`, `"1.1 GB"`.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

/// Shared interface for command result structs, providing text and JSON rendering.
///
/// Every command collects its results into a struct that implements this trait.
/// JSON output is derived automatically via `Serialize`; commands only need to
/// implement `format_text`.
pub trait CommandOutput: Serialize {
    /// Render this result as human-readable text (tables, summaries).
    /// When `verbose` is true, output includes expanded details and a metadata footer.
    fn format_text(&self, verbose: bool) -> String;

    /// Render this result as JSON. Default serializes the full struct.
    /// Command output wrappers override this to omit `process` in compact mode.
    fn format_json(&self, verbose: bool) -> String {
        let _ = verbose;
        serde_json::to_string_pretty(self).expect("CommandOutput types must be JSON-serializable")
    }

    /// Print to stdout in the requested format.
    /// Default implementation handles dispatch — commands don't need to override this.
    fn print(&self, format: &OutputFormat, verbose: bool) {
        match format {
            OutputFormat::Text => print!("{}", self.format_text(verbose)),
            OutputFormat::Json => print!("{}", self.format_json(verbose)),
        }
    }
}

/// Serialize command output as JSON, omitting process steps in compact mode.
///
/// In compact mode (not verbose), only the result is emitted: `{"result": ...}`.
/// In verbose mode or when the result is absent (error), the full struct is serialized.
pub fn format_json_compact<T: Serialize, R: Serialize>(
    full: &T,
    result: Option<&R>,
    verbose: bool,
) -> String {
    if result.is_some() && !verbose {
        serde_json::to_string_pretty(&serde_json::json!({ "result": result }))
            .expect("CommandOutput types must be JSON-serializable")
    } else {
        serde_json::to_string_pretty(full).expect("CommandOutput types must be JSON-serializable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_file_count_singular() {
        assert_eq!(format_file_count(1), "1 file");
    }

    #[test]
    fn format_file_count_plural() {
        assert_eq!(format_file_count(0), "0 files");
        assert_eq!(format_file_count(5), "5 files");
    }

    #[test]
    fn format_size_units() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1_048_576), "1.0 MB");
        assert_eq!(format_size(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn json_serialization_roundtrip() {
        let field = DiscoveredField {
            name: "title".into(),
            field_type: "String".into(),
            files_found: 5,
            total_files: 10,
            allowed: Some(vec!["**".into()]),
            required: None,
            nullable: false,
            hints: vec![],
        };
        let json = serde_json::to_string(&field).unwrap();
        assert!(json.contains("\"title\""));
        assert!(json.contains("\"String\""));
        assert!(!json.contains("required")); // skip_serializing_if = None
        assert!(!json.contains("hints")); // skip_serializing_if = Vec::is_empty
    }

    #[test]
    fn json_serialization_with_hints() {
        let field = DiscoveredField {
            name: "author's".into(),
            field_type: "String".into(),
            files_found: 1,
            total_files: 1,
            allowed: None,
            required: None,
            nullable: false,
            hints: vec![FieldHint::EscapeSingleQuotes],
        };
        let json = serde_json::to_string(&field).unwrap();
        assert!(json.contains("\"hints\""));
        assert!(json.contains("escape single quotes"));
    }

    #[test]
    fn field_hints_no_special_chars() {
        assert!(field_hints("title").is_empty());
    }

    #[test]
    fn field_hints_single_quote() {
        let hints = field_hints("author's_note");
        assert_eq!(hints, vec![FieldHint::EscapeSingleQuotes]);
    }

    #[test]
    fn field_hints_double_quote() {
        let hints = field_hints("field\"name");
        assert_eq!(hints, vec![FieldHint::EscapeDoubleQuotes]);
    }

    #[test]
    fn field_hints_both_quotes() {
        let hints = field_hints("it's a \"test\"");
        assert_eq!(hints.len(), 2);
        assert!(hints.contains(&FieldHint::EscapeSingleQuotes));
        assert!(hints.contains(&FieldHint::EscapeDoubleQuotes));
    }

    #[test]
    fn format_hints_empty() {
        assert_eq!(format_hints(&[]), "");
    }

    #[test]
    fn format_hints_single() {
        let s = format_hints(&[FieldHint::EscapeSingleQuotes]);
        assert!(s.contains("'"));
        assert!(s.contains("''"));
    }

    #[test]
    fn format_hints_multiple() {
        let s = format_hints(&[FieldHint::EscapeSingleQuotes, FieldHint::EscapeDoubleQuotes]);
        assert!(s.contains(", "));
    }

    #[test]
    fn field_hint_serde_roundtrip() {
        let hints = vec![FieldHint::EscapeSingleQuotes, FieldHint::EscapeDoubleQuotes];
        let json = serde_json::to_string(&hints).unwrap();
        let parsed: Vec<FieldHint> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, hints);
    }
}
