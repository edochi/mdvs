use crate::output::{format_file_count, format_json_compact, format_size, CommandOutput};
use crate::pipeline::delete_index::DeleteIndexOutput;
use crate::pipeline::ProcessingStepResult;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Result of the `clean` command.
#[derive(Debug, Serialize)]
pub struct CleanResult {
    /// Whether the `.mdvs/` directory was actually removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: PathBuf,
    /// Number of files that were in `.mdvs/` before deletion.
    pub files_removed: usize,
    /// Total size of `.mdvs/` in bytes before deletion.
    pub size_bytes: u64,
}

impl CommandOutput for CleanResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();
        if self.removed {
            out.push_str(&format!("Cleaned \"{}\"\n", self.path.display()));
            if verbose {
                out.push_str(&format!(
                    "\n{} | {}\n",
                    format_file_count(self.files_removed),
                    format_size(self.size_bytes),
                ));
            }
        } else {
            out.push_str(&format!(
                "Nothing to clean — \"{}\" does not exist\n",
                self.path.display()
            ));
        }
        out
    }
}

// ============================================================================
// CleanCommandOutput (pipeline-based)
// ============================================================================

/// Pipeline record for the clean command.
#[derive(Debug, Serialize)]
pub struct CleanProcessOutput {
    /// Delete index step result.
    pub delete_index: ProcessingStepResult<DeleteIndexOutput>,
}

/// Full output of the clean command: pipeline steps + command result.
#[derive(Debug, Serialize)]
pub struct CleanCommandOutput {
    /// Processing steps and their outcomes.
    pub process: CleanProcessOutput,
    /// Command result (None if pipeline didn't complete).
    pub result: Option<CleanResult>,
}

impl CleanCommandOutput {
    /// Returns `true` if any processing step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.delete_index, ProcessingStepResult::Failed(_))
    }
}

impl CommandOutput for CleanCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        if let Some(result) = &self.result {
            if verbose {
                let mut out = String::new();
                out.push_str(&format!("{}\n", self.process.delete_index.format_line()));
                out.push('\n');
                out.push_str(&result.format_text(verbose));
                out
            } else {
                result.format_text(verbose)
            }
        } else {
            // Pipeline failed — show the step error
            format!("{}\n", self.process.delete_index.format_line())
        }
    }
}

/// Delete the `.mdvs/` index directory if it exists.
#[instrument(name = "clean", skip_all)]
pub fn run(path: &Path) -> CleanCommandOutput {
    use crate::pipeline::delete_index::run_delete_index;

    let delete_step = run_delete_index(path);

    let result = match &delete_step {
        ProcessingStepResult::Completed(step) => Some(CleanResult {
            removed: step.output.removed,
            path: PathBuf::from(&step.output.path),
            files_removed: step.output.files_removed,
            size_bytes: step.output.size_bytes,
        }),
        _ => None,
    };

    CleanCommandOutput {
        process: CleanProcessOutput {
            delete_index: delete_step,
        },
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn clean_removes_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();

        // Create mdvs.toml and .mdvs/ with a dummy file
        fs::write(tmp.path().join("mdvs.toml"), "[scan]\nglob = \"**\"\n").unwrap();
        let mdvs_dir = tmp.path().join(".mdvs");
        fs::create_dir_all(&mdvs_dir).unwrap();
        fs::write(mdvs_dir.join("files.parquet"), "dummy").unwrap();

        let output = run(tmp.path());
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(result.removed);
        assert!(!mdvs_dir.exists());
        assert_eq!(result.files_removed, 1);
        assert_eq!(result.size_bytes, 5); // "dummy" is 5 bytes
                                          // mdvs.toml should be untouched
        assert!(tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn clean_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();

        let output = run(tmp.path());
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(!result.removed);
        assert_eq!(result.files_removed, 0);
        assert_eq!(result.size_bytes, 0);
    }
}
