use serde::Serialize;
use std::path::PathBuf;

/// Controls whether command output is rendered as human-readable text or machine-readable JSON.
#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pretty-printed tables and summaries for terminal display.
    Human,
    /// Structured JSON for piping into other tools.
    Json,
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
}

/// A field whose inferred type changed between the previous and current scan.
#[derive(Debug, Serialize)]
pub struct ChangedField {
    /// Field name.
    pub name: String,
    /// Type recorded in the existing `mdvs.toml`.
    pub old_type: String,
    /// Newly inferred type after re-scanning.
    pub new_type: String,
}

/// Category of a frontmatter validation failure.
#[derive(Debug, Clone, Serialize)]
pub enum ViolationKind {
    /// A field marked `required` is absent from the file's frontmatter.
    MissingRequired,
    /// The field's value does not match the expected type.
    WrongType,
    /// The field is not declared in `mdvs.toml` and is not in the ignore list.
    Disallowed,
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
}

/// Shared interface for command result structs, providing human and JSON rendering.
///
/// Every command collects its results into a struct that implements this trait.
/// JSON output is derived automatically via `Serialize`; commands only need to
/// implement `format_human`.
pub trait CommandOutput: Serialize {
    /// Render this result as human-readable text (tables, summaries).
    fn format_human(&self) -> String;

    /// Print to stdout in the requested format.
    /// Default implementation handles dispatch — commands don't need to override this.
    fn print(&self, format: &OutputFormat) {
        match format {
            OutputFormat::Human => print!("{}", self.format_human()),
            OutputFormat::Json => print!("{}", serde_json::to_string_pretty(self).unwrap()),
        }
    }
}
