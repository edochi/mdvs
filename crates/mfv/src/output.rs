use crate::diagnostic::Diagnostic;

/// Output format for diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    Github,
}

/// Format diagnostics as a string in the given format.
pub fn format_diagnostics(diagnostics: &[Diagnostic], format: OutputFormat) -> String {
    match format {
        OutputFormat::Human => format_human(diagnostics),
        OutputFormat::Json => format_json(diagnostics),
        OutputFormat::Github => format_github(diagnostics),
    }
}

fn format_human(diagnostics: &[Diagnostic]) -> String {
    if diagnostics.is_empty() {
        return "All files valid.".to_string();
    }

    let mut out = String::new();
    let mut current_file = "";

    for d in diagnostics {
        if d.file != current_file {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&d.file);
            out.push('\n');
            current_file = &d.file;
        }
        out.push_str(&format!("  - {}: {}\n", d.field, d.kind));
    }

    let file_count = {
        let mut files: Vec<&str> = diagnostics.iter().map(|d| d.file.as_str()).collect();
        files.dedup();
        files.len()
    };

    out.push_str(&format!(
        "\n{} error(s) in {} file(s)\n",
        diagnostics.len(),
        file_count
    ));

    out
}

fn format_json(diagnostics: &[Diagnostic]) -> String {
    let items: Vec<serde_json::Value> = diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "file": d.file,
                "field": d.field,
                "message": d.kind.to_string(),
            })
        })
        .collect();

    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".to_string())
}

fn format_github(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|d| format!("::error file={}::field '{}': {}", d.file, d.field, d.kind))
        .collect::<Vec<_>>()
        .join("\n")
}
