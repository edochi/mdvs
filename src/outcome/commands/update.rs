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

        for field in &self.added {
            let mut detail_lines = Vec::new();
            if let Some(ref globs) = field.allowed {
                detail_lines.push("  found in:".to_string());
                for g in globs {
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
                        "added".to_string(),
                        field.field_type.clone(),
                    ],
                    vec![detail_lines.join("\n"), String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }

        for field in &self.changed {
            let mut rows = vec![vec![
                "field".into(),
                "aspect".into(),
                "old".into(),
                "new".into(),
            ]];
            for (i, change) in field.changes.iter().enumerate() {
                let name_col = if i == 0 {
                    format!("\"{}\"", field.name)
                } else {
                    String::new()
                };
                let (old, new) = change.format_old_new();
                rows.push(vec![name_col, change.label().to_string(), old, new]);
            }
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::Compact,
            });
        }

        for field in &self.removed {
            let detail = match &field.allowed {
                Some(globs) => {
                    let mut lines = vec!["  previously in:".to_string()];
                    for g in globs {
                        lines.push(format!("    - \"{g}\""));
                    }
                    lines.join("\n")
                }
                None => String::new(),
            };
            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![
                        format!("\"{}\"", field.name),
                        "removed".to_string(),
                        String::new(),
                    ],
                    vec![detail, String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }

        blocks
    }
}
