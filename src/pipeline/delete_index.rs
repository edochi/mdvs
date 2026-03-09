//! Delete index step — removes the `.mdvs/` directory.

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

use crate::index::backend::Backend;
use crate::output::{format_file_count, format_size};
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};

/// Output record for the delete index step.
#[derive(Debug, Serialize)]
pub struct DeleteIndexOutput {
    /// Whether the `.mdvs/` directory existed and was removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: String,
    /// Number of files removed.
    pub files_removed: usize,
    /// Total bytes freed.
    pub size_bytes: u64,
}

impl StepOutput for DeleteIndexOutput {
    fn format_line(&self) -> String {
        if self.removed {
            format!(
                "\"{}\" ({}, {})",
                self.path,
                format_file_count(self.files_removed),
                format_size(self.size_bytes)
            )
        } else {
            format!("\"{}\" does not exist", self.path)
        }
    }
}

/// Count files and sum their sizes in a directory (recursively).
fn walk_dir_stats(dir: &Path) -> anyhow::Result<(usize, u64)> {
    let mut count = 0usize;
    let mut size = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            let (c, s) = walk_dir_stats(&entry.path())?;
            count += c;
            size += s;
        } else {
            count += 1;
            size += meta.len();
        }
    }
    Ok((count, size))
}

/// Delete the `.mdvs/` index directory if it exists.
///
/// Returns just `ProcessingStepResult<DeleteIndexOutput>` — there is no data
/// to pass forward to subsequent steps.
pub fn run_delete_index(path: &Path) -> ProcessingStepResult<DeleteIndexOutput> {
    let start = Instant::now();
    let mdvs_dir = path.join(".mdvs");

    // Symlink check → User error
    if mdvs_dir.is_symlink() {
        return ProcessingStepResult::Failed(ProcessingStepError {
            kind: ErrorKind::User,
            message: format!(
                "'{}' is a symlink — refusing to delete for safety",
                mdvs_dir.display()
            ),
        });
    }

    if mdvs_dir.exists() {
        let (files_removed, size_bytes) = match walk_dir_stats(&mdvs_dir) {
            Ok(stats) => stats,
            Err(e) => {
                return ProcessingStepResult::Failed(ProcessingStepError {
                    kind: ErrorKind::Application,
                    message: e.to_string(),
                });
            }
        };

        let backend = Backend::parquet(path, "_");
        if let Err(e) = backend.clean() {
            return ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            });
        }

        ProcessingStepResult::Completed(ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: DeleteIndexOutput {
                removed: true,
                path: mdvs_dir.display().to_string(),
                files_removed,
                size_bytes,
            },
        })
    } else {
        ProcessingStepResult::Completed(ProcessingStep {
            elapsed_ms: start.elapsed().as_millis() as u64,
            output: DeleteIndexOutput {
                removed: false,
                path: mdvs_dir.display().to_string(),
                files_removed: 0,
                size_bytes: 0,
            },
        })
    }
}
