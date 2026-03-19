//! Init command outcome types.

use std::path::PathBuf;

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::{format_file_count, format_hints, DiscoveredField, DiscoveredFieldCompact};

/// Full outcome for the init command.
#[derive(Debug, Serialize)]
pub struct InitOutcome {
    /// Directory where `mdvs.toml` was written.
    pub path: PathBuf,
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Fields inferred from frontmatter.
    pub fields: Vec<DiscoveredField>,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
}

impl Render for InitOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // One-liner
        let field_summary = if self.fields.is_empty() {
            "no fields found".to_string()
        } else {
            format!("{} field(s)", self.fields.len())
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        blocks.push(Block::Line(format!(
            "Initialized {} — {field_summary}{dry_run_suffix}",
            format_file_count(self.files_scanned)
        )));

        // Per-field record tables
        for field in &self.fields {
            let mut detail_lines = Vec::new();
            if let Some(ref req) = field.required {
                if !req.is_empty() {
                    detail_lines.push("  required:".to_string());
                    for g in req {
                        detail_lines.push(format!("    - \"{g}\""));
                    }
                }
            }
            if let Some(ref allowed) = field.allowed {
                detail_lines.push("  allowed:".to_string());
                for g in allowed {
                    detail_lines.push(format!("    - \"{g}\""));
                }
            }
            if field.nullable {
                detail_lines.push("  nullable: true".to_string());
            }
            if !field.hints.is_empty() {
                detail_lines.push(format!("  hints: {}", format_hints(&field.hints)));
            }

            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![
                        format!("\"{}\"", field.name),
                        field.field_type.clone(),
                        format!("{}/{}", field.files_found, field.total_files),
                    ],
                    vec![detail_lines.join("\n"), String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }

        // Footer
        if self.dry_run {
            blocks.push(Block::Line("(dry run, nothing written)".into()));
        } else {
            blocks.push(Block::Line(format!(
                "Initialized mdvs in '{}'",
                self.path.display()
            )));
        }

        blocks
    }
}

/// Compact outcome for the init command.
#[derive(Debug, Serialize)]
pub struct InitOutcomeCompact {
    /// Directory where `mdvs.toml` was written.
    pub path: PathBuf,
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Number of fields inferred.
    pub field_count: usize,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// Compact field summaries.
    pub fields: Vec<DiscoveredFieldCompact>,
}

impl Render for InitOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        let field_summary = if self.fields.is_empty() {
            "no fields found".to_string()
        } else {
            format!("{} field(s)", self.field_count)
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        blocks.push(Block::Line(format!(
            "Initialized {} — {field_summary}{dry_run_suffix}",
            format_file_count(self.files_scanned)
        )));

        // Compact fields table
        if !self.fields.is_empty() {
            let rows: Vec<Vec<String>> = self
                .fields
                .iter()
                .map(|f| {
                    let type_str = if f.nullable {
                        format!("{}?", f.field_type)
                    } else {
                        f.field_type.clone()
                    };
                    vec![
                        format!("\"{}\"", f.name),
                        type_str,
                        format!("{}/{}", f.files_found, f.total_files),
                    ]
                })
                .collect();
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::Compact,
            });
        }

        if self.dry_run {
            blocks.push(Block::Line("(dry run, nothing written)".into()));
        } else {
            blocks.push(Block::Line(format!(
                "Initialized mdvs in '{}'",
                self.path.display()
            )));
        }

        blocks
    }
}

impl From<&InitOutcome> for InitOutcomeCompact {
    fn from(o: &InitOutcome) -> Self {
        Self {
            path: o.path.clone(),
            files_scanned: o.files_scanned,
            field_count: o.fields.len(),
            dry_run: o.dry_run,
            fields: o.fields.iter().map(DiscoveredFieldCompact::from).collect(),
        }
    }
}
