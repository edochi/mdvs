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

        // Summary line
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
        blocks.push(Block::Line(String::new()));

        // Config section (always shown)
        let ignored_str = if self.ignored_fields.is_empty() {
            "(none)".into()
        } else {
            self.ignored_fields.join("\n")
        };
        blocks.push(Block::Line("Config:".into()));
        blocks.push(Block::Table {
            headers: None,
            rows: vec![
                vec!["scan glob".into(), self.scan_glob.clone()],
                vec!["ignored fields".into(), ignored_str],
            ],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        });

        // Index metadata
        if let Some(idx) = &self.index {
            let rev = idx.revision.as_deref().unwrap_or("none");
            let rows = vec![
                vec!["model".into(), idx.model.clone()],
                vec!["revision".into(), rev.to_string()],
                vec!["chunk size".into(), idx.chunk_size.to_string()],
                vec!["built".into(), idx.built_at.clone()],
                vec!["config".into(), idx.config_status.clone()],
                vec![
                    "files".into(),
                    format!("{} out of {}", idx.files_indexed, idx.files_on_disk),
                ],
            ];
            blocks.push(Block::Line("Index:".into()));
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::KeyValue {
                    title: String::new(),
                },
            });
        }

        // Per-field KeyValue tables
        if !self.fields.is_empty() {
            blocks.push(Block::Line(format!("{} fields:", self.fields.len())));
        }
        for f in &self.fields {
            let files_str = match (f.count, f.total_files) {
                (Some(c), Some(t)) => format!("{c} out of {t}"),
                _ => String::new(),
            };

            let mut rows = vec![
                vec!["type".into(), f.field_type.clone()],
                vec!["files".into(), files_str],
                vec!["nullable".into(), f.nullable.to_string()],
            ];

            let req_val = if f.required.is_empty() {
                "(none)".into()
            } else {
                f.required.join("\n")
            };
            rows.push(vec!["required".into(), req_val]);

            let allow_val = if f.allowed.is_empty() {
                "**".into()
            } else {
                f.allowed.join("\n")
            };
            rows.push(vec!["allowed".into(), allow_val]);

            if !f.hints.is_empty() {
                rows.push(vec!["hints".into(), format_hints(&f.hints)]);
            }

            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::KeyValue {
                    title: f.name.clone(),
                },
            });
        }

        blocks
    }
}
