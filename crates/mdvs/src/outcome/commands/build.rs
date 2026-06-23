//! Build command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::BuildFileDetail;
use crate::output::{NewField, format_file_count};

fn format_chunk_count(n: usize) -> String {
    if n == 1 {
        "1 chunk".to_string()
    } else {
        format!("{n} chunks")
    }
}

/// Full outcome for the build command.
#[derive(Debug, Serialize)]
pub struct BuildOutcome {
    /// Whether this was a full rebuild (vs incremental).
    pub full_rebuild: bool,
    /// Total number of files in the final index.
    pub files_total: usize,
    /// Number of files that were chunked and embedded this run.
    pub files_embedded: usize,
    /// Number of files reused from the previous index.
    pub files_unchanged: usize,
    /// Number of files removed since the last build.
    pub files_removed: usize,
    /// Total number of chunks in the final index.
    pub chunks_total: usize,
    /// Number of chunks produced by newly embedded files.
    pub chunks_embedded: usize,
    /// Number of chunks retained from unchanged files.
    pub chunks_unchanged: usize,
    /// Number of chunks dropped from removed files.
    pub chunks_removed: usize,
    /// Fields found in frontmatter but not yet in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
    /// Per-file chunk counts for embedded files.
    pub embedded_files: Vec<BuildFileDetail>,
    /// Per-file chunk counts for removed files.
    pub removed_files: Vec<BuildFileDetail>,
}

impl Render for BuildOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
        let rebuild_suffix = if self.full_rebuild {
            " (full rebuild)"
        } else {
            ""
        };
        blocks.push(Block::Line(format!(
            "Built index — {}, {}{rebuild_suffix}",
            format_file_count(self.files_total),
            format_chunk_count(self.chunks_total)
        )));
        blocks.push(Block::Line(String::new()));

        // All JSON fields as key-value rows
        let new_fields_str = if self.new_fields.is_empty() {
            "(none)".into()
        } else {
            self.new_fields
                .iter()
                .map(|nf| format!("{} ({})", nf.name, format_file_count(nf.files_found)))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let rows = vec![
            vec!["full rebuild".into(), self.full_rebuild.to_string()],
            vec!["files total".into(), self.files_total.to_string()],
            vec!["files embedded".into(), self.files_embedded.to_string()],
            vec!["files unchanged".into(), self.files_unchanged.to_string()],
            vec!["files removed".into(), self.files_removed.to_string()],
            vec!["chunks total".into(), self.chunks_total.to_string()],
            vec!["chunks embedded".into(), self.chunks_embedded.to_string()],
            vec!["chunks unchanged".into(), self.chunks_unchanged.to_string()],
            vec!["chunks removed".into(), self.chunks_removed.to_string()],
            vec!["new fields".into(), new_fields_str],
        ];

        blocks.push(Block::Table {
            headers: None,
            rows,
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        });

        // Per-file lists go in their own Section blocks. Stuffing a 500-row
        // file list into a key-value table cell renders as a single
        // <br>-joined string in markdown and a giant tabled cell in the
        // terminal — neither is readable. A Section gives both formats a
        // proper heading + one-line-per-file structure.
        if !self.embedded_files.is_empty() {
            blocks.push(Block::Section {
                label: "embedded files".into(),
                children: self
                    .embedded_files
                    .iter()
                    .map(|f| {
                        Block::Line(format!("{} ({})", f.filename, format_chunk_count(f.chunks)))
                    })
                    .collect(),
            });
        }

        if !self.removed_files.is_empty() {
            blocks.push(Block::Section {
                label: "removed files".into(),
                children: self
                    .removed_files
                    .iter()
                    .map(|f| {
                        Block::Line(format!("{} ({})", f.filename, format_chunk_count(f.chunks)))
                    })
                    .collect(),
            });
        }

        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::{format_markdown, format_pretty};

    fn outcome_with_two_files() -> BuildOutcome {
        BuildOutcome {
            full_rebuild: true,
            files_total: 2,
            files_embedded: 2,
            files_unchanged: 0,
            files_removed: 0,
            chunks_total: 3,
            chunks_embedded: 3,
            chunks_unchanged: 0,
            chunks_removed: 0,
            new_fields: vec![],
            embedded_files: vec![
                BuildFileDetail {
                    filename: "notes/alpha.md".into(),
                    chunks: 2,
                },
                BuildFileDetail {
                    filename: "notes/beta.md".into(),
                    chunks: 1,
                },
            ],
            removed_files: vec![],
        }
    }

    /// Per-file lists must not land in the key-value summary table — that
    /// produced an unreadable <br>-joined GFM cell with hundreds of files
    /// jammed into it. They belong in their own Section.
    #[test]
    fn embedded_files_render_as_section_not_table_cell() {
        let blocks = outcome_with_two_files().render();

        // Summary table must NOT contain an "embedded files" row.
        let summary_row_cells: Vec<&str> = blocks
            .iter()
            .filter_map(|b| match b {
                Block::Table { rows, .. } => Some(rows),
                _ => None,
            })
            .flatten()
            .map(|row| row[0].as_str())
            .collect();
        assert!(
            !summary_row_cells.contains(&"embedded files"),
            "summary table still has an 'embedded files' row: {summary_row_cells:?}"
        );
        assert!(
            !summary_row_cells.contains(&"removed files"),
            "summary table still has a 'removed files' row: {summary_row_cells:?}"
        );

        // A Section labeled "embedded files" must exist with one child per file.
        let section = blocks
            .iter()
            .find_map(|b| match b {
                Block::Section { label, children } if label == "embedded files" => Some(children),
                _ => None,
            })
            .expect("expected an 'embedded files' Section");
        assert_eq!(section.len(), 2);
    }

    /// Markdown output must put each file on its own line under a `##`
    /// heading, NOT inside a `<br>`-joined table cell.
    #[test]
    fn markdown_renders_embedded_files_as_heading_and_lines() {
        let md = format_markdown(&outcome_with_two_files().render());
        assert!(
            md.contains("## embedded files"),
            "expected '## embedded files' heading, got:\n{md}"
        );
        assert!(md.contains("notes/alpha.md (2 chunks)"));
        assert!(md.contains("notes/beta.md (1 chunk)"));
        assert!(
            !md.contains("notes/alpha.md (2 chunks)<br>notes/beta.md"),
            "file list leaked back into a <br>-joined table cell:\n{md}"
        );
    }

    /// Pretty output must put each file on its own indented line under the
    /// section header.
    #[test]
    fn pretty_renders_embedded_files_as_indented_list() {
        let pretty = format_pretty(&outcome_with_two_files().render());
        assert!(pretty.contains("embedded files:"));
        assert!(pretty.contains("notes/alpha.md (2 chunks)"));
        assert!(pretty.contains("notes/beta.md (1 chunk)"));
    }

    /// Empty file lists do NOT emit empty Section blocks (no `## embedded
    /// files` heading with nothing under it).
    #[test]
    fn empty_file_lists_emit_no_section() {
        let mut outcome = outcome_with_two_files();
        outcome.embedded_files.clear();
        outcome.removed_files.clear();
        let blocks = outcome.render();
        assert!(
            !blocks
                .iter()
                .any(|b| matches!(b, Block::Section { label, .. } if label == "embedded files" || label == "removed files")),
            "empty file lists should not produce Section blocks"
        );
    }
}
