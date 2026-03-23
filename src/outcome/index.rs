//! Outcome types for index-related leaf steps (DeleteIndex, ReadIndex, WriteIndex).

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
}
