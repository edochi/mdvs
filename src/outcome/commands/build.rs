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

        // New fields (shown before stats)
        for nf in &self.new_fields {
            blocks.push(Block::Line(format!(
                "  new field: {} ({})",
                nf.name,
                format_file_count(nf.files_found)
            )));
        }
        if !self.new_fields.is_empty() {
            blocks.push(Block::Line(
                "Run 'mdvs update' to incorporate new fields.".into(),
            ));
        }

        // One-liner
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

        // Verbose: record tables per category with file-by-file detail
        if self.files_embedded > 0 {
            let detail = self
                .embedded_files
                .iter()
                .map(|f| format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks)))
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![
                        "embedded".to_string(),
                        format_file_count(self.files_embedded),
                        format_chunk_count(self.chunks_embedded),
                    ],
                    vec![detail, String::new(), String::new()],
                ],
                style: TableStyle::Record {
                    detail_rows: vec![1],
                },
            });
        }
        if self.files_unchanged > 0 {
            blocks.push(Block::Table {
                headers: None,
                rows: vec![vec![
                    "unchanged".to_string(),
                    format_file_count(self.files_unchanged),
                    format_chunk_count(self.chunks_unchanged),
                ]],
                style: TableStyle::Compact,
            });
        }
        if self.files_removed > 0 {
            let detail = self
                .removed_files
                .iter()
                .map(|f| format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks)))
                .collect::<Vec<_>>()
                .join("\n");
            blocks.push(Block::Table {
                headers: None,
                rows: vec![
                    vec![
                        "removed".to_string(),
                        format_file_count(self.files_removed),
                        format_chunk_count(self.chunks_removed),
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

/// Compact outcome for the build command.
#[derive(Debug, Serialize)]
pub struct BuildOutcomeCompact {
    /// Whether this was a full rebuild.
    pub full_rebuild: bool,
    /// Total files in the final index.
    pub files_total: usize,
    /// Files embedded this run.
    pub files_embedded: usize,
    /// Files unchanged from previous build.
    pub files_unchanged: usize,
    /// Files removed since last build.
    pub files_removed: usize,
    /// Total chunks in the final index.
    pub chunks_total: usize,
    /// Chunks produced by new embeddings.
    pub chunks_embedded: usize,
    /// Chunks retained from unchanged files.
    pub chunks_unchanged: usize,
    /// Chunks dropped from removed files.
    pub chunks_removed: usize,
}

impl Render for BuildOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

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

        // Compact stats table
        let mut rows = vec![];
        if self.files_embedded > 0 {
            rows.push(vec![
                "embedded".to_string(),
                format_file_count(self.files_embedded),
                format_chunk_count(self.chunks_embedded),
            ]);
        }
        if self.files_unchanged > 0 {
            rows.push(vec![
                "unchanged".to_string(),
                format_file_count(self.files_unchanged),
                format_chunk_count(self.chunks_unchanged),
            ]);
        }
        if self.files_removed > 0 {
            rows.push(vec![
                "removed".to_string(),
                format_file_count(self.files_removed),
                format_chunk_count(self.chunks_removed),
            ]);
        }
        if !rows.is_empty() {
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::Compact,
            });
        }

        blocks
    }
}

impl From<&BuildOutcome> for BuildOutcomeCompact {
    fn from(o: &BuildOutcome) -> Self {
        Self {
            full_rebuild: o.full_rebuild,
            files_total: o.files_total,
            files_embedded: o.files_embedded,
            files_unchanged: o.files_unchanged,
            files_removed: o.files_removed,
            chunks_total: o.chunks_total,
            chunks_embedded: o.chunks_embedded,
            chunks_unchanged: o.chunks_unchanged,
            chunks_removed: o.chunks_removed,
        }
    }
}
