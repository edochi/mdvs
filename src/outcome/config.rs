//! Outcome types for config-related leaf steps (ReadConfig, WriteConfig).

use serde::Serialize;

use crate::block::{Block, Render};

/// Full outcome for the read_config step.
#[derive(Debug, Serialize)]
pub struct ReadConfigOutcome {
    /// Path to the config file that was read.
    pub config_path: String,
}

impl Render for ReadConfigOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!("Read config: {}", self.config_path))]
    }
}

/// Full outcome for the write_config step.
#[derive(Debug, Serialize)]
pub struct WriteConfigOutcome {
    /// Path to the config file that was written.
    pub config_path: String,
    /// Number of fields written to the config.
    pub fields_written: usize,
}

impl Render for WriteConfigOutcome {
    fn render(&self) -> Vec<Block> {
        vec![Block::Line(format!("Write config: {}", self.config_path))]
    }
}
