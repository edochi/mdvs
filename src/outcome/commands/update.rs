//! Update command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::{format_file_count, format_hints, ChangedField, DiscoveredField, RemovedField};

/// Full outcome for the update command.
#[derive(Debug, Serialize)]
pub struct UpdateOutcome {
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Newly discovered fields not previously in `mdvs.toml`.
    pub added: Vec<DiscoveredField>,
    /// Fields whose type or glob constraints changed during re-inference.
    pub changed: Vec<ChangedField>,
    /// Fields that disappeared from all files during re-inference.
    pub removed: Vec<RemovedField>,
    /// Number of fields that remained identical.
    pub unchanged: usize,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
}

impl UpdateOutcome {
    fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.changed.is_empty() || !self.removed.is_empty()
    }
}

impl Render for UpdateOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
        let total_changes = self.added.len() + self.changed.len() + self.removed.len();
        let summary = if total_changes == 0 {
            "no changes".to_string()
        } else {
            format!("{total_changes} field(s) changed")
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        blocks.push(Block::Line(format!(
            "Scanned {} — {summary}{dry_run_suffix}",
            format_file_count(self.files_scanned)
        )));

        if !self.has_changes() {
            return blocks;
        }

        // Added fields — same KeyValue format as init
        if !self.added.is_empty() {
            blocks.push(Block::Line(String::new()));
            blocks.push(Block::Line(format!("Added ({}):", self.added.len())));
            for field in &self.added {
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
        }

        // Changed fields — one row per changed aspect with old → new
        if !self.changed.is_empty() {
            blocks.push(Block::Line(String::new()));
            blocks.push(Block::Line(format!("Changed ({}):", self.changed.len())));
            for field in &self.changed {
                let rows: Vec<Vec<String>> = field
                    .changes
                    .iter()
                    .map(|c| {
                        let (old, new) = c.format_old_new();
                        vec![c.label().to_string(), format!("{old} \u{2192} {new}")]
                    })
                    .collect();
                blocks.push(Block::Table {
                    headers: None,
                    rows,
                    style: TableStyle::KeyValue {
                        title: field.name.clone(),
                    },
                });
            }
        }

        // Removed fields
        if !self.removed.is_empty() {
            blocks.push(Block::Line(String::new()));
            blocks.push(Block::Line(format!("Removed ({}):", self.removed.len())));
            for field in &self.removed {
                let rows = match &field.allowed {
                    Some(globs) if !globs.is_empty() => {
                        vec![vec!["previously allowed".into(), globs.join("\n")]]
                    }
                    _ => vec![vec!["status".into(), "removed".into()]],
                };
                blocks.push(Block::Table {
                    headers: None,
                    rows,
                    style: TableStyle::KeyValue {
                        title: field.name.clone(),
                    },
                });
            }
        }

        blocks
    }
}
