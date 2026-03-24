//! Init command outcome types.

use std::path::PathBuf;

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::{format_file_count, format_hints, DiscoveredField};

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

/// Check if a field has only default constraints (allowed: ** only, not nullable, no required).
fn has_non_default_constraints(field: &DiscoveredField) -> bool {
    let has_required = field.required.as_ref().is_some_and(|r| !r.is_empty());
    let has_non_default_allowed = field
        .allowed
        .as_ref()
        .is_some_and(|a| !(a.len() == 1 && a[0] == "**"));
    has_required || has_non_default_allowed || field.nullable || !field.hints.is_empty()
}

/// Build detail lines for a field's constraints (only non-default values).
fn field_detail_lines(field: &DiscoveredField) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(ref req) = field.required {
        if !req.is_empty() {
            lines.push("  required:".to_string());
            for g in req {
                lines.push(format!("    {g}"));
            }
        }
    }
    if let Some(ref allowed) = field.allowed {
        // Skip if only "**" (the default)
        if !(allowed.len() == 1 && allowed[0] == "**") {
            lines.push("  allowed:".to_string());
            for g in allowed {
                lines.push(format!("    {g}"));
            }
        }
    }
    if field.nullable {
        lines.push("  nullable: true".to_string());
    }
    if !field.hints.is_empty() {
        lines.push(format!("  hints: {}", format_hints(&field.hints)));
    }
    lines
}

impl Render for InitOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
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

        // Per-field tables
        for field in &self.fields {
            let data_row = vec![
                field.name.clone(),
                field.field_type.clone(),
                format!("{}/{}", field.files_found, field.total_files),
            ];

            if has_non_default_constraints(field) {
                let detail = field_detail_lines(field).join("\n");
                blocks.push(Block::Table {
                    headers: None,
                    rows: vec![data_row, vec![detail, String::new(), String::new()]],
                    style: TableStyle::Record {
                        detail_rows: vec![1],
                    },
                });
            } else {
                blocks.push(Block::Table {
                    headers: None,
                    rows: vec![data_row],
                    style: TableStyle::Record {
                        detail_rows: vec![],
                    },
                });
            }
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
