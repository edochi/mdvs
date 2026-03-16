//! Write config step — constructs and writes `mdvs.toml` from inferred schema.

use serde::Serialize;
use std::path::Path;
use std::time::Instant;

use crate::discover::infer::InferredSchema;
use crate::pipeline::{
    ErrorKind, ProcessingStep, ProcessingStepError, ProcessingStepResult, StepOutput,
};
use crate::schema::config::MdvsToml;
use crate::schema::shared::ScanConfig;

/// Output record for the write_config step.
#[derive(Debug, Serialize)]
pub struct WriteConfigOutput {
    /// Path where `mdvs.toml` was written.
    pub config_path: String,
    /// Number of fields written to the config.
    pub fields_written: usize,
}

impl StepOutput for WriteConfigOutput {
    fn format_line(&self) -> String {
        self.config_path.clone()
    }
}

/// Construct `MdvsToml` from inferred schema and write to disk.
/// Schema-only — no build sections are written.
pub fn run_write_config(
    path: &Path,
    schema: &InferredSchema,
    scan_config: ScanConfig,
) -> (ProcessingStepResult<WriteConfigOutput>, Option<MdvsToml>) {
    let start = Instant::now();
    let config_path = path.join("mdvs.toml");

    let toml_doc = MdvsToml::from_inferred(schema, scan_config);

    if let Err(e) = toml_doc.write(&config_path) {
        return (
            ProcessingStepResult::Failed(ProcessingStepError {
                kind: ErrorKind::Application,
                message: e.to_string(),
            }),
            None,
        );
    }

    let fields_written = schema.fields.len();
    let step = ProcessingStep {
        elapsed_ms: start.elapsed().as_millis() as u64,
        output: WriteConfigOutput {
            config_path: config_path.display().to_string(),
            fields_written,
        },
    };
    (ProcessingStepResult::Completed(step), Some(toml_doc))
}
