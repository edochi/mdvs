//! Read config step — loads and parses `mdvs.toml`.

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};
use crate::schema::config::MdvsToml;

/// Output record for the read config step.
#[derive(Debug, Serialize)]
pub struct ReadConfigOutput {
    /// Path to the config file that was read.
    pub config_path: String,
}

impl StepOutput for ReadConfigOutput {
    fn format_line(&self) -> String {
        self.config_path.clone()
    }
}

/// Read and parse `mdvs.toml` from the given project path.
///
/// Returns the step result (for the pipeline record) and the parsed config
/// (for the next step to consume). The config is `None` if reading failed.
pub fn run_read_config(path: &Path) -> (ProcessingStepResult<ReadConfigOutput>, Option<MdvsToml>) {
    let start = Instant::now();
    let config_path = path.join("mdvs.toml");
    match MdvsToml::read(&config_path) {
        Ok(config) => {
            if let Err(e) = config.validate() {
                let err = ProcessingStepError {
                    kind: ErrorKind::User,
                    message: format!(
                        "mdvs.toml is invalid: {} — fix the file or run 'mdvs init --force'",
                        e
                    ),
                };
                return (ProcessingStepResult::Failed(err), None);
            }
            let step = ProcessingStep {
                elapsed_ms: start.elapsed().as_millis() as u64,
                output: ReadConfigOutput {
                    config_path: config_path.display().to_string(),
                },
            };
            (ProcessingStepResult::Completed(step), Some(config))
        }
        Err(e) => {
            let err = ProcessingStepError {
                kind: ErrorKind::User,
                message: e.to_string(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
    }
}
