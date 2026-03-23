//! Check command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::{format_file_count, FieldViolation, NewField, ViolationKind};

/// Full outcome for the check command.
#[derive(Debug, Serialize)]
pub struct CheckOutcome {
    /// Number of markdown files checked.
    pub files_checked: usize,
    /// Violations grouped by field and kind.
    pub violations: Vec<FieldViolation>,
    /// Fields found in frontmatter but not defined in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
}

impl Render for CheckOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        let violation_part = if self.violations.is_empty() {
            "no violations".to_string()
        } else {
            format!("{} violation(s)", self.violations.len())
        };
        let new_field_part = if self.new_fields.is_empty() {
            String::new()
        } else {
            format!(", {} new field(s)", self.new_fields.len())
        };
        blocks.push(Block::Line(format!(
            "Checked {} — {violation_part}{new_field_part}",
            format_file_count(self.files_checked),
        )));

        for v in &self.violations {
            let kind_str = match v.kind {
                ViolationKind::MissingRequired => "MissingRequired",
                ViolationKind::WrongType => "WrongType",
                ViolationKind::Disallowed => "Disallowed",
                ViolationKind::NullNotAllowed => "NullNotAllowed",
            };
            let detail_text = v
                .files
                .iter()
                .map(|f| match &f.detail {
                    Some(d) => format!("  - \"{}\" ({d})", f.path.display()),
                    None => format!("  - \"{}\"", f.path.display()),
                })
                .collect::<Vec<_>>()
                .join("\n");

            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![
                        format!("\"{}\"", v.field),
                        kind_str.to_string(),
                        format_file_count(v.files.len()),
                    ],
                    vec![detail_text, String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }

        for nf in &self.new_fields {
            let detail_text = match &nf.files {
                Some(files) => files
                    .iter()
                    .map(|p| format!("  - \"{}\"", p.display()))
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => String::new(),
            };
            let mut rows = vec![vec![
                format!("\"{}\"", nf.name),
                "new".to_string(),
                format_file_count(nf.files_found),
            ]];
            if !detail_text.is_empty() {
                rows.push(vec![detail_text, String::new(), String::new()]);
            }
            blocks.push(Block::Table {
                headers: None,
                rows: rows.clone(),
                style: if rows.len() > 1 {
                    TableStyle::Record {
                        detail_rows: vec![1],
                    }
                } else {
                    TableStyle::Compact
                },
            });
        }

        blocks
    }
}
