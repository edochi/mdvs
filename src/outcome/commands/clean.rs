//! Clean command outcome types.

use std::path::PathBuf;

use serde::Serialize;

use crate::block::{Block, Render};
use crate::output::{format_file_count, format_size};

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
        if self.removed {
            vec![
                Block::Line(format!("Cleaned \"{}\"", self.path.display())),
                Block::Line(format!(
                    "{} | {}",
                    format_file_count(self.files_removed),
                    format_size(self.size_bytes),
                )),
            ]
        } else {
            vec![Block::Line(format!(
                "Nothing to clean — \"{}\" does not exist",
                self.path.display()
            ))]
        }
    }
}

/// Compact outcome for the clean command.
#[derive(Debug, Serialize)]
pub struct CleanOutcomeCompact {
    /// Whether `.mdvs/` was actually removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: PathBuf,
}

impl Render for CleanOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        if self.removed {
            vec![Block::Line(format!("Cleaned \"{}\"", self.path.display()))]
        } else {
            vec![Block::Line(format!(
                "Nothing to clean — \"{}\" does not exist",
                self.path.display()
            ))]
        }
    }
}

impl From<&CleanOutcome> for CleanOutcomeCompact {
    fn from(o: &CleanOutcome) -> Self {
        Self {
            removed: o.removed,
            path: o.path.clone(),
        }
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
        assert_eq!(blocks.len(), 2);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Cleaned \".mdvs\""),
            _ => panic!("expected Line"),
        }
        match &blocks[1] {
            Block::Line(s) => assert!(s.contains("2 files") && s.contains("1.0 KB")),
            _ => panic!("expected Line"),
        }
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
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert!(s.contains("Nothing to clean")),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn clean_compact_removed() {
        let outcome = CleanOutcomeCompact {
            removed: true,
            path: PathBuf::from(".mdvs"),
        };
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Cleaned \".mdvs\""),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn clean_compact_from_full() {
        let full = CleanOutcome {
            removed: true,
            path: PathBuf::from(".mdvs"),
            files_removed: 5,
            size_bytes: 4096,
        };
        let compact = CleanOutcomeCompact::from(&full);
        assert!(compact.removed);
        assert_eq!(compact.path, PathBuf::from(".mdvs"));
    }
}
