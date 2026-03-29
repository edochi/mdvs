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

/// Human-readable violation kind name.
fn kind_display(kind: &ViolationKind) -> &'static str {
    match kind {
        ViolationKind::MissingRequired => "Missing required",
        ViolationKind::WrongType => "Wrong type",
        ViolationKind::Disallowed => "Not allowed",
        ViolationKind::NullNotAllowed => "Null value not allowed",
    }
}

impl Render for CheckOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
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

        // Violations section
        if !self.violations.is_empty() {
            blocks.push(Block::Line(String::new()));
            blocks.push(Block::Line(format!(
                "Violations ({}):",
                self.violations.len()
            )));
            for v in &self.violations {
                let files_str = v
                    .files
                    .iter()
                    .map(|f| match &f.detail {
                        Some(d) => format!("{} ({})", f.path.display(), d),
                        None => f.path.display().to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("\n");

                let rows = vec![
                    vec!["kind".into(), kind_display(&v.kind).into()],
                    vec!["rule".into(), v.rule.clone()],
                    vec!["files".into(), files_str],
                ];
                blocks.push(Block::Table {
                    headers: None,
                    rows,
                    style: TableStyle::KeyValue {
                        title: v.field.clone(),
                    },
                });
            }
        }

        // New fields section
        if !self.new_fields.is_empty() {
            blocks.push(Block::Line(String::new()));
            blocks.push(Block::Line(format!(
                "New fields ({}):",
                self.new_fields.len()
            )));
            for nf in &self.new_fields {
                let found_in = match &nf.files {
                    Some(files) if !files.is_empty() => files
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => format_file_count(nf.files_found),
                };
                let rows = vec![
                    vec!["status".into(), "new (not in mdvs.toml)".into()],
                    vec!["found in".into(), found_in],
                ];
                blocks.push(Block::Table {
                    headers: None,
                    rows,
                    style: TableStyle::KeyValue {
                        title: nf.name.clone(),
                    },
                });
            }
        }

        blocks
    }
}
