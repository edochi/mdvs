use serde::Serialize;
use std::path::PathBuf;

/// Controls whether command output is rendered as plain text or machine-readable JSON.
#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    /// Pretty-printed tables and summaries for terminal display.
    Text,
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

    /// Print to stdout in the requested format.
    /// Default implementation handles dispatch — commands don't need to override this.
    fn print(&self, format: &OutputFormat, verbose: bool) {
        match format {
            OutputFormat::Text => print!("{}", self.format_text(verbose)),
            OutputFormat::Json => print!("{}", serde_json::to_string_pretty(self).unwrap()),
        }
    }
}
