//! Outcome types for config-related leaf steps (ReadConfig, WriteConfig, etc.).
//!
//! Only ReadConfig is defined initially. WriteConfig, MutateConfig, and
//! CheckConfigChanged are added when init/build commands are converted.

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

/// Compact outcome for the read_config step (identical — no verbose-only fields).
#[derive(Debug, Serialize)]
pub struct ReadConfigOutcomeCompact {
    /// Path to the config file that was read.
    pub config_path: String,
}

impl Render for ReadConfigOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&ReadConfigOutcome> for ReadConfigOutcomeCompact {
    fn from(o: &ReadConfigOutcome) -> Self {
        Self {
            config_path: o.config_path.clone(),
        }
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

/// Compact outcome for the write_config step (identical — no verbose-only fields).
#[derive(Debug, Serialize)]
pub struct WriteConfigOutcomeCompact {
    /// Path to the config file that was written.
    pub config_path: String,
    /// Number of fields written to the config.
    pub fields_written: usize,
}

impl Render for WriteConfigOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        vec![] // Leaf compact outcomes are silent
    }
}

impl From<&WriteConfigOutcome> for WriteConfigOutcomeCompact {
    fn from(o: &WriteConfigOutcome) -> Self {
        Self {
            config_path: o.config_path.clone(),
            fields_written: o.fields_written,
        }
    }
}
