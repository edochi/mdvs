//! Export-jsonschema command outcome.

use std::path::PathBuf;

use serde::Serialize;

use crate::block::{Block, Render};

/// Output format for the exported schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
#[value(rename_all = "lowercase")]
pub enum ExportFormat {
    /// Pretty-printed JSON.
    Json,
    /// TOML encoding of the JSON Schema (via `tomljson`).
    Toml,
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportFormat::Json => f.write_str("json"),
            ExportFormat::Toml => f.write_str("toml"),
        }
    }
}

/// Full outcome for the export-jsonschema command.
#[derive(Debug, Serialize)]
pub struct ExportJsonschemaOutcome {
    /// Path of the `mdvs.toml` that was read.
    pub source: PathBuf,
    /// Where the schema was written. `None` means stdout.
    pub destination: Option<PathBuf>,
    /// Format of the emitted schema.
    pub format: ExportFormat,
    /// Number of `[[fields.field]]` entries exported.
    pub fields_exported: usize,
    /// Number of `[fields].ignore` entries exported as empty schemas.
    pub ignore_exported: usize,
}

impl Render for ExportJsonschemaOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];
        let dest = match &self.destination {
            Some(p) => format!("to '{}'", p.display()),
            None => "to stdout".to_string(),
        };
        blocks.push(Block::Line(format!(
            "Exported {} field(s) + {} ignored ({}) {dest}.",
            self.fields_exported, self.ignore_exported, self.format
        )));
        blocks
    }
}
