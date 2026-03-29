//! Clean command outcome types.

use std::path::PathBuf;

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::output::format_size;

/// Full outcome for the clean command.
#[derive(Debug, Serialize)]
pub struct CleanOutcome {
    /// Whether `.mdvs/` was actually removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: PathBuf,
    /// Number of files that were in `.mdvs/`.
    pub files_removed: usize,
    /// Total size of `.mdvs/` in bytes.
    pub size_bytes: u64,
}

impl Render for CleanOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
        if self.removed {
            blocks.push(Block::Line(format!("Cleaned \"{}\"", self.path.display())));
        } else {
            blocks.push(Block::Line(format!(
                "Nothing to clean — \"{}\" does not exist",
                self.path.display()
            )));
        }
        blocks.push(Block::Line(String::new()));

        // All JSON fields as key-value rows
        let rows = vec![
            vec!["removed".into(), self.removed.to_string()],
            vec!["path".into(), self.path.display().to_string()],
            vec!["files removed".into(), self.files_removed.to_string()],
            vec!["size".into(), format_size(self.size_bytes)],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_render_removed() {
        let outcome = CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 2,
            size_bytes: 1024,
        };
        let blocks = outcome.render();
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Cleaned \".mdvs\""),
            _ => panic!("expected Line"),
        }
        // Table is present
        assert!(blocks.iter().any(|b| matches!(b, Block::Table { .. })));
    }

    #[test]
    fn clean_render_nothing() {
        let outcome = CleanOutcome {
            removed: false,
            path: PathBuf::from(".mdvs"),
            files_removed: 0,
            size_bytes: 0,
        };
        let blocks = outcome.render();
        match &blocks[0] {
            Block::Line(s) => assert!(s.contains("Nothing to clean")),
            _ => panic!("expected Line"),
        }
    }
}
