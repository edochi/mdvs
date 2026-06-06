//! Classification step of the build pipeline.
//!
//! Compares the current scan against the existing on-disk index to partition
//! files into three groups: new/edited (need embedding), unchanged (keep
//! existing chunks), and removed (drop existing chunks). The result feeds
//! the embed and write steps in [`super::build_core`].

use crate::discover::scan::{ScannedFile, ScannedFiles};
use crate::index::storage::{ChunkRow, FileIndexEntry, content_hash};
use crate::output::BuildFileDetail;
use std::collections::{HashMap, HashSet};

/// A file that needs chunking and embedding.
pub(super) struct FileToEmbed<'a> {
    /// Unique file identifier (preserved for edited files, new UUID for new files).
    pub(super) file_id: String,
    /// Reference to the scanned file data.
    pub(super) scanned: &'a ScannedFile,
}

/// Data produced by classification, carried forward to embed and write_index steps.
pub(super) struct ClassifyData<'a> {
    /// Whether this is a full rebuild.
    pub(super) full_rebuild: bool,
    /// Files that need chunking + embedding (new or edited).
    pub(super) needs_embedding: Vec<FileToEmbed<'a>>,
    /// Maps filename → file_id for ALL current files (new, edited, unchanged).
    pub(super) file_id_map: HashMap<String, String>,
    /// Chunks retained from unchanged files.
    pub(super) retained_chunks: Vec<ChunkRow>,
    /// Number of files removed since previous build.
    pub(super) removed_count: usize,
    /// Number of chunks dropped from removed files.
    pub(super) chunks_removed: usize,
    /// Per-file chunk counts for removed files (for verbose output).
    pub(super) removed_details: Vec<BuildFileDetail>,
    /// file_ids of files that were removed (chunks to delete in the
    /// incremental write path).
    pub(super) removed_file_ids: Vec<String>,
}

pub(super) struct FileClassification<'a> {
    pub(super) needs_embedding: Vec<FileToEmbed<'a>>,
    pub(super) file_id_map: HashMap<String, String>,
    pub(super) unchanged_file_ids: HashSet<String>,
    pub(super) removed_count: usize,
    pub(super) removed_file_ids: HashSet<String>,
    pub(super) removed_filenames: Vec<String>,
}

pub(super) fn classify_files<'a>(
    scanned: &'a ScannedFiles,
    existing_index: &[FileIndexEntry],
) -> FileClassification<'a> {
    let existing: HashMap<&str, (&str, &str)> = existing_index
        .iter()
        .map(|e| {
            (
                e.filename.as_str(),
                (e.file_id.as_str(), e.content_hash.as_str()),
            )
        })
        .collect();

    let mut needs_embedding = Vec::new();
    let mut file_id_map = HashMap::new();
    let mut unchanged_file_ids = HashSet::new();
    let mut seen_filenames = HashSet::new();

    for file in &scanned.files {
        let filename = file.path.display().to_string();
        let hash = content_hash(&file.content);

        if let Some(&(old_id, old_hash)) = existing.get(filename.as_str()) {
            seen_filenames.insert(filename.clone());
            if hash == old_hash {
                file_id_map.insert(filename, old_id.to_string());
                unchanged_file_ids.insert(old_id.to_string());
            } else {
                let file_id = old_id.to_string();
                file_id_map.insert(filename, file_id.clone());
                needs_embedding.push(FileToEmbed {
                    file_id,
                    scanned: file,
                });
            }
        } else {
            let file_id = uuid::Uuid::new_v4().to_string();
            file_id_map.insert(filename, file_id.clone());
            needs_embedding.push(FileToEmbed {
                file_id,
                scanned: file,
            });
        }
    }

    let mut removed_file_ids = HashSet::new();
    let mut removed_filenames = Vec::new();
    for entry in existing_index {
        if !seen_filenames.contains(entry.filename.as_str()) {
            removed_file_ids.insert(entry.file_id.clone());
            removed_filenames.push(entry.filename.clone());
        }
    }
    let removed_count = removed_filenames.len();

    FileClassification {
        needs_embedding,
        file_id_map,
        unchanged_file_ids,
        removed_count,
        removed_file_ids,
        removed_filenames,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::scan::ScannedFile;

    fn make_scanned_files(files: Vec<(&str, &str)>) -> ScannedFiles {
        ScannedFiles {
            files: files
                .into_iter()
                .map(|(path, body)| ScannedFile {
                    path: std::path::PathBuf::from(path),
                    data: None,
                    content: body.to_string(),
                    body_line_offset: 0,
                    frontmatter_error: None,
                })
                .collect(),
        }
    }

    #[test]
    fn classify_all_new() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing: Vec<FileIndexEntry> = vec![];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 2);
        assert_eq!(c.unchanged_file_ids.len(), 0);
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map.len(), 2);
    }

    #[test]
    fn classify_all_unchanged() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("hello"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("world"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 0);
        assert_eq!(c.unchanged_file_ids.len(), 2);
        assert!(c.unchanged_file_ids.contains("f1"));
        assert!(c.unchanged_file_ids.contains("f2"));
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map["a.md"], "f1");
        assert_eq!(c.file_id_map["b.md"], "f2");
    }

    #[test]
    fn classify_mixed() {
        let scanned = make_scanned_files(vec![
            ("a.md", "same content"),
            ("b.md", "new body"),
            ("c.md", "brand new"),
        ]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("same content"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("old body"),
            },
            FileIndexEntry {
                file_id: "f3".into(),
                filename: "d.md".into(),
                content_hash: content_hash("deleted"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        assert!(c.unchanged_file_ids.contains("f1"));
        assert_eq!(c.file_id_map["a.md"], "f1");

        assert_eq!(c.needs_embedding.len(), 2);
        let edited = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("b.md"))
            .unwrap();
        assert_eq!(edited.file_id, "f2");

        let new = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("c.md"))
            .unwrap();
        assert_ne!(new.file_id, "f1");
        assert_ne!(new.file_id, "f2");
        assert_ne!(new.file_id, "f3");

        assert_eq!(c.removed_count, 1);
        assert!(!c.file_id_map.contains_key("d.md"));
    }
}
