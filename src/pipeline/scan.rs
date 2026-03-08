//! Scan step — walks the filesystem and parses frontmatter.

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

use crate::discover::scan::ScannedFiles;
use crate::output::format_file_count;
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};
use crate::schema::shared::ScanConfig;

/// Output record for the scan step.
#[derive(Debug, Serialize)]
pub struct ScanOutput {
    /// Number of markdown files found.
    pub files_found: usize,
    /// Glob pattern used for scanning.
    pub glob: String,
}

impl StepOutput for ScanOutput {
    fn format_line(&self) -> String {
        format!("Scanned {}", format_file_count(self.files_found))
    }
}

/// Scan the project directory for markdown files and parse their frontmatter.
///
/// Returns the step result (for the pipeline record) and the scanned files
/// (for the next step to consume). The scanned files are `None` if scanning failed.
pub fn run_scan(
    path: &Path,
    config: &ScanConfig,
) -> (ProcessingStepResult<ScanOutput>, Option<ScannedFiles>) {
    let start = Instant::now();
    match ScannedFiles::scan(path, config) {
        Ok(scanned) => {
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ScanOutput {
                    files_found: scanned.files.len(),
                    glob: config.glob.clone(),
                },
            };
            (ProcessingStepResult::Completed(step), Some(scanned))
        }
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
    }
}
