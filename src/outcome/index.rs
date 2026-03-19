//! Outcome types for index-related leaf steps (DeleteIndex, ReadIndex, etc.).
//!
//! Only DeleteIndex is defined initially. Other index outcomes are added
//! incrementally as commands are converted.

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::{format_file_count, format_size};

/// Full outcome for the delete_index step.
#[derive(Debug, Serialize)]
pub struct DeleteIndexOutcome {
    /// Whether `.mdvs/` existed and was removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: String,
    /// Number of files removed.
    pub files_removed: usize,
    /// Total bytes freed.
    pub size_bytes: u64,
}

impl Render for DeleteIndexOutcome {
    fn render(&self) -> Vec<Block> {
        if self.removed {
            vec![Block::Line(format!(
                "Delete index: {} ({}, {})",
                self.path,
                format_file_count(self.files_removed),
                format_size(self.size_bytes),
            ))]
        } else {
            vec![Block::Line(format!(
                "Delete index: {} does not exist",
                self.path
            ))]
        }
    }
}

/// Compact outcome for the delete_index step (identical fields — leaf step).
#[derive(Debug, Serialize)]
pub struct DeleteIndexOutcomeCompact {
    /// Whether `.mdvs/` existed and was removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: String,
    /// Number of files removed.
    pub files_removed: usize,
    /// Total bytes freed.
    pub size_bytes: u64,
}

impl Render for DeleteIndexOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&DeleteIndexOutcome> for DeleteIndexOutcomeCompact {
    fn from(o: &DeleteIndexOutcome) -> Self {
        Self {
            removed: o.removed,
            path: o.path.clone(),
            files_removed: o.files_removed,
            size_bytes: o.size_bytes,
        }
    }
}

/// Full outcome for the read_index step.
#[derive(Debug, Serialize)]
pub struct ReadIndexOutcome {
    /// Whether the index exists.
    pub exists: bool,
    /// Number of files in the index (0 if not exists).
    pub files_indexed: usize,
    /// Number of chunks in the index (0 if not exists).
    pub chunks: usize,
}

impl Render for ReadIndexOutcome {
    fn render(&self) -> Vec<Block> {
        if self.exists {
            vec![Block::Line(format!(
                "Read index: {} files, {} chunks",
                self.files_indexed, self.chunks
            ))]
        } else {
            vec![Block::Line("Read index: not found".into())]
        }
    }
}

/// Compact outcome for the read_index step (identical — no verbose-only fields).
#[derive(Debug, Serialize)]
pub struct ReadIndexOutcomeCompact {
    /// Whether the index exists.
    pub exists: bool,
    /// Number of files in the index.
    pub files_indexed: usize,
    /// Number of chunks in the index.
    pub chunks: usize,
}

impl Render for ReadIndexOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&ReadIndexOutcome> for ReadIndexOutcomeCompact {
    fn from(o: &ReadIndexOutcome) -> Self {
        Self {
            exists: o.exists,
            files_indexed: o.files_indexed,
            chunks: o.chunks,
        }
    }
}

/// Full outcome for the write_index step.
#[derive(Debug, Serialize)]
pub struct WriteIndexOutcome {
    /// Number of files written.
    pub files_written: usize,
    /// Number of chunks written.
    pub chunks_written: usize,
}

impl Render for WriteIndexOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!(
            "Write index: {} files, {} chunks",
            self.files_written, self.chunks_written
        ))]
    }
}

/// Compact outcome for the write_index step (identical).
#[derive(Debug, Serialize)]
pub struct WriteIndexOutcomeCompact {
    /// Number of files written.
    pub files_written: usize,
    /// Number of chunks written.
    pub chunks_written: usize,
}

impl Render for WriteIndexOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![]
    }
}

impl From<&WriteIndexOutcome> for WriteIndexOutcomeCompact {
    fn from(o: &WriteIndexOutcome) -> Self {
        Self {
            files_written: o.files_written,
            chunks_written: o.chunks_written,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_index_render_removed() {
        let outcome = DeleteIndexOutcome {
            removed: true,
            path: ".mdvs".into(),
            files_removed: 2,
            size_bytes: 1024,
        };
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert!(s.contains(".mdvs") && s.contains("2 files")),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn delete_index_render_not_exists() {
        let outcome = DeleteIndexOutcome {
            removed: false,
            path: ".mdvs".into(),
            files_removed: 0,
            size_bytes: 0,
        };
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert!(s.contains("does not exist")),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn delete_index_compact_is_silent() {
        let outcome = DeleteIndexOutcomeCompact {
            removed: true,
            path: ".mdvs".into(),
            files_removed: 2,
            size_bytes: 1024,
        };
        assert!(outcome.render().is_empty());
    }

    #[test]
    fn delete_index_from_full() {
        let full = DeleteIndexOutcome {
            removed: true,
            path: ".mdvs".into(),
            files_removed: 3,
            size_bytes: 2048,
        };
        let compact = DeleteIndexOutcomeCompact::from(&full);
        assert_eq!(compact.removed, true);
        assert_eq!(compact.path, ".mdvs");
        assert_eq!(compact.files_removed, 3);
        assert_eq!(compact.size_bytes, 2048);
    }
}
