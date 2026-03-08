//! Classify step — classifies files as new/edited/unchanged/removed for incremental builds.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

use crate::discover::scan::{ScannedFile, ScannedFiles};
use crate::index::storage::{content_hash, ChunkRow, FileIndexEntry};
use crate::output::format_file_count;
use crate::pipeline::{ProcessingStep, ProcessingStepResult, StepOutput};

use super::write_index::BuildFileDetail;

/// Output record for the classify step.
#[derive(Debug, Serialize)]
pub struct ClassifyOutput {
    /// Whether this is a full rebuild (force or first build).
    pub full_rebuild: bool,
    /// Number of files that need embedding (new + edited).
    pub needs_embedding: usize,
    /// Number of files unchanged from previous build.
    pub unchanged: usize,
    /// Number of files removed since previous build.
    pub removed: usize,
}

impl StepOutput for ClassifyOutput {
    fn format_line(&self) -> String {
        if self.full_rebuild {
            format!(
                "Classified {} (full rebuild)",
                format_file_count(self.needs_embedding)
            )
        } else {
            format!(
                "Classified {} ({} to embed, {} unchanged, {} removed)",
                format_file_count(self.needs_embedding + self.unchanged),
                self.needs_embedding,
                self.unchanged,
                self.removed
            )
        }
    }
}

/// A file that needs chunking and embedding.
pub(crate) struct FileToEmbed<'a> {
    /// Unique file identifier (preserved for edited files, new UUID for new files).
    pub file_id: String,
    /// Reference to the scanned file data.
    pub scanned: &'a ScannedFile,
}

/// Data produced by classification, carried forward to embed and write_index steps.
pub(crate) struct ClassifyData<'a> {
    /// Whether this is a full rebuild.
    pub full_rebuild: bool,
    /// Files that need chunking + embedding (new or edited).
    pub needs_embedding: Vec<FileToEmbed<'a>>,
    /// Maps filename → file_id for ALL current files (new, edited, unchanged).
    pub file_id_map: HashMap<String, String>,
    /// Chunks retained from unchanged files.
    pub retained_chunks: Vec<ChunkRow>,
    /// Number of files removed since previous build.
    pub removed_count: usize,
    /// Number of chunks dropped from removed files.
    pub chunks_removed: usize,
    /// Per-file chunk counts for removed files (for verbose output).
    pub removed_details: Vec<BuildFileDetail>,
}

/// Classify files for incremental build.
///
/// For full rebuilds (`force` or no existing index), all scanned files go to
/// `needs_embedding`. For incremental builds, files are compared against the
/// existing index by content hash.
///
/// Returns the step result and the classification data for subsequent steps.
pub(crate) fn run_classify<'a>(
    scanned: &'a ScannedFiles,
    existing_index: &[FileIndexEntry],
    existing_chunks: Vec<ChunkRow>,
    full_rebuild: bool,
) -> (
    ProcessingStepResult<ClassifyOutput>,
    Option<ClassifyData<'a>>,
) {
    let start = Instant::now();

    if full_rebuild {
        // Full rebuild: all files need embedding, no retained chunks
        let mut file_id_map = HashMap::new();
        let needs_embedding: Vec<FileToEmbed<'a>> = scanned
            .files
            .iter()
            .map(|f| {
                let file_id = uuid::Uuid::new_v4().to_string();
                let filename = f.path.display().to_string();
                file_id_map.insert(filename, file_id.clone());
                FileToEmbed {
                    file_id,
                    scanned: f,
                }
            })
            .collect();

        let count = needs_embedding.len();
        let step = ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: ClassifyOutput {
                full_rebuild: true,
                needs_embedding: count,
                unchanged: 0,
                removed: 0,
            },
        };
        let data = ClassifyData {
            full_rebuild: true,
            needs_embedding,
            file_id_map,
            retained_chunks: vec![],
            removed_count: 0,
            chunks_removed: 0,
            removed_details: vec![],
        };
        (ProcessingStepResult::Completed(step), Some(data))
    } else {
        // Incremental: classify by comparing content hashes
        let classification = classify_files(scanned, existing_index);

        // Count removed chunks and build removed file details
        let mut removed_chunk_counts: HashMap<&str, usize> = HashMap::new();
        for c in &existing_chunks {
            if classification.removed_file_ids.contains(&c.file_id) {
                *removed_chunk_counts.entry(c.file_id.as_str()).or_default() += 1;
            }
        }
        let chunks_removed: usize = removed_chunk_counts.values().sum();

        // Build removed file details (map file_id back to filename)
        let filename_to_id: HashMap<&str, &str> = existing_index
            .iter()
            .map(|e| (e.filename.as_str(), e.file_id.as_str()))
            .collect();
        let mut removed_details = Vec::new();
        for filename in &classification.removed_filenames {
            let file_id = filename_to_id.get(filename.as_str()).copied().unwrap_or("");
            let chunk_count = removed_chunk_counts.get(file_id).copied().unwrap_or(0);
            removed_details.push(BuildFileDetail {
                filename: filename.clone(),
                chunks: chunk_count,
            });
        }

        // Retain chunks from unchanged files
        let retained_chunks: Vec<ChunkRow> = existing_chunks
            .into_iter()
            .filter(|c| classification.unchanged_file_ids.contains(&c.file_id))
            .collect();

        let needs_count = classification.needs_embedding.len();
        let unchanged_count = classification.unchanged_file_ids.len();
        let removed_count = classification.removed_count;

        let step = ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: ClassifyOutput {
                full_rebuild: false,
                needs_embedding: needs_count,
                unchanged: unchanged_count,
                removed: removed_count,
            },
        };
        let data = ClassifyData {
            full_rebuild: false,
            needs_embedding: classification.needs_embedding,
            file_id_map: classification.file_id_map,
            retained_chunks,
            removed_count,
            chunks_removed,
            removed_details,
        };
        (ProcessingStepResult::Completed(step), Some(data))
    }
}

struct FileClassification<'a> {
    /// Files that need chunking + embedding (new or edited).
    needs_embedding: Vec<FileToEmbed<'a>>,
    /// Maps filename → file_id for ALL current files (new, edited, unchanged).
    file_id_map: HashMap<String, String>,
    /// file_ids whose existing chunks should be retained.
    unchanged_file_ids: HashSet<String>,
    /// Number of files in the old index that no longer exist.
    removed_count: usize,
    /// file_ids of removed files (for chunk counting).
    removed_file_ids: HashSet<String>,
    /// Filenames of removed files (for verbose output).
    removed_filenames: Vec<String>,
}

fn classify_files<'a>(
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
                // Unchanged — keep existing chunks
                file_id_map.insert(filename, old_id.to_string());
                unchanged_file_ids.insert(old_id.to_string());
            } else {
                // Edited — re-embed, keep file_id
                let file_id = old_id.to_string();
                file_id_map.insert(filename, file_id.clone());
                needs_embedding.push(FileToEmbed {
                    file_id,
                    scanned: file,
                });
            }
        } else {
            // New file
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
        // a.md: unchanged, b.md: edited, c.md: new, d.md: removed
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

        // a.md unchanged
        assert!(c.unchanged_file_ids.contains("f1"));
        assert_eq!(c.file_id_map["a.md"], "f1");

        // b.md edited — needs embedding, keeps file_id
        assert_eq!(c.needs_embedding.len(), 2); // b.md + c.md
        let edited = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("b.md"))
            .unwrap();
        assert_eq!(edited.file_id, "f2");

        // c.md new — needs embedding, new UUID
        let new = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("c.md"))
            .unwrap();
        assert_ne!(new.file_id, "f1");
        assert_ne!(new.file_id, "f2");
        assert_ne!(new.file_id, "f3");

        // d.md removed
        assert_eq!(c.removed_count, 1);
        assert!(!c.file_id_map.contains_key("d.md"));
    }
}
