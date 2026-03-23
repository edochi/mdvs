//! Info command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::cmd::info::{IndexInfo, InfoField};
use crate::output::format_hints;

/// Full outcome for the info command.
#[derive(Debug, Serialize)]
pub struct InfoOutcome {
    /// Glob pattern from `[scan]` config.
    pub scan_glob: String,
    /// Number of markdown files matching the scan pattern.
    pub files_on_disk: usize,
    /// Field definitions from `[[fields.field]]`.
    pub fields: Vec<InfoField>,
    /// Field names in the `[fields].ignore` list.
    pub ignored_fields: Vec<String>,
    /// Index info, if a built index exists.
    pub index: Option<IndexInfo>,
}

impl Render for InfoOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        let one_liner = match &self.index {
            Some(idx) => format!(
                "{} files, {} fields, {} chunks",
                self.files_on_disk,
                self.fields.len(),
                idx.chunks,
            ),
            None => format!("{} files, {} fields", self.files_on_disk, self.fields.len()),
        };
        blocks.push(Block::Line(one_liner));

        if let Some(idx) = &self.index {
            let rev = idx.revision.as_deref().unwrap_or("none");
            let rows = vec![
                vec!["model:".into(), idx.model.clone()],
                vec!["revision:".into(), rev.to_string()],
                vec!["chunk size:".into(), idx.chunk_size.to_string()],
                vec!["built:".into(), idx.built_at.clone()],
                vec!["config:".into(), idx.config_status.clone()],
                vec![
                    "files:".into(),
                    format!("{}/{}", idx.files_indexed, idx.files_on_disk),
                ],
            ];
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::Compact,
            });
        }

        for f in &self.fields {
            let count_str = match (f.count, f.total_files) {
                (Some(c), Some(t)) => format!("{c}/{t}"),
                _ => String::new(),
            };
            let mut detail_lines = Vec::new();
            if !f.required.is_empty() {
                detail_lines.push("  required:".to_string());
                for g in &f.required {
                    detail_lines.push(format!("    - \"{g}\""));
                }
            }
            detail_lines.push("  allowed:".to_string());
            for g in &f.allowed {
                detail_lines.push(format!("    - \"{g}\""));
            }
            if f.nullable {
                detail_lines.push("  nullable: true".to_string());
            }
            if !f.hints.is_empty() {
                detail_lines.push(format!("  hints: {}", format_hints(&f.hints)));
            }

            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![format!("\"{}\"", f.name), f.field_type.clone(), count_str],
                    vec![detail_lines.join("\n"), String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }

        blocks
    }
}
