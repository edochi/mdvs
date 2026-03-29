//! Build command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::BuildFileDetail;
use crate::output::{format_file_count, NewField};

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

        let embedded_files_str = if self.embedded_files.is_empty() {
            "(none)".into()
        } else {
            self.embedded_files
                .iter()
                .map(|f| format!("{} ({})", f.filename, format_chunk_count(f.chunks)))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let removed_files_str = if self.removed_files.is_empty() {
            "(none)".into()
        } else {
            self.removed_files
                .iter()
                .map(|f| format!("{} ({})", f.filename, format_chunk_count(f.chunks)))
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
            vec!["embedded files".into(), embedded_files_str],
            vec!["removed files".into(), removed_files_str],
        ];

        blocks.push(Block::Table {
            headers: None,
            rows,
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        });

        blocks
    }
}
