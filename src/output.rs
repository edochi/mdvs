use serde::Serialize;

#[derive(Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Serialize)]
pub struct DiscoveredField {
    pub name: String,
    pub field_type: String,
    pub files_found: usize,
    pub total_files: usize,
}

#[derive(Debug, Serialize)]
pub struct ChangedField {
    pub name: String,
    pub old_type: String,
    pub new_type: String,
}

pub trait CommandOutput: Serialize {
    /// Render this result as human-readable text (tables, summaries).
    fn format_human(&self) -> String;

    /// Print to stdout in the requested format.
    /// Default implementation handles dispatch — commands don't need to override this.
    fn print(&self, format: &OutputFormat) {
        match format {
            OutputFormat::Human => print!("{}", self.format_human()),
            OutputFormat::Json => print!("{}", serde_json::to_string_pretty(self).unwrap()),
        }
    }
}
