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

        // Per-field key-value tables
        for field in &self.fields {
            let mut rows = vec![
                vec!["type".into(), field.field_type.clone()],
                vec![
                    "files".into(),
                    format!("{} out of {}", field.files_found, field.total_files),
                ],
                vec!["nullable".into(), field.nullable.to_string()],
            ];

            let req_val = match &field.required {
                Some(r) if !r.is_empty() => r.join("\n"),
                _ => "(none)".into(),
            };
            rows.push(vec!["required".into(), req_val]);

            let allow_val = match &field.allowed {
                Some(a) if !a.is_empty() => a.join("\n"),
                _ => "**".into(),
            };
            rows.push(vec!["allowed".into(), allow_val]);

            if !field.hints.is_empty() {
                rows.push(vec!["hints".into(), format_hints(&field.hints)]);
            }

            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::KeyValue {
                    title: field.name.clone(),
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
