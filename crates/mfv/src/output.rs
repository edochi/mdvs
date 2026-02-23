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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostic::DiagnosticKind;

    fn diag(file: &str, field: &str, kind: DiagnosticKind) -> Diagnostic {
        Diagnostic {
            file: file.to_string(),
            field: field.to_string(),
            kind,
        }
    }

    #[test]
    fn human_single_error() {
        let diagnostics = vec![diag("test.md", "title", DiagnosticKind::MissingRequired)];
        let out = format_diagnostics(&diagnostics, OutputFormat::Human);
        assert!(out.contains("test.md"));
        assert!(out.contains("  - title: required field missing"));
        assert!(out.contains("1 error(s) in 1 file(s)"));
    }

    #[test]
    fn human_multiple_errors_same_file() {
        let diagnostics = vec![
            diag("test.md", "title", DiagnosticKind::MissingRequired),
            diag(
                "test.md",
                "date",
                DiagnosticKind::WrongType {
                    expected: "date".to_string(),
                    got: "string".to_string(),
                },
            ),
        ];
        let out = format_diagnostics(&diagnostics, OutputFormat::Human);
        // File header should appear only once
        assert_eq!(out.matches("test.md").count(), 1);
        assert!(out.contains("2 error(s) in 1 file(s)"));
    }

    #[test]
    fn human_multiple_files() {
        let diagnostics = vec![
            diag("a.md", "title", DiagnosticKind::MissingRequired),
            diag("b.md", "date", DiagnosticKind::MissingRequired),
        ];
        let out = format_diagnostics(&diagnostics, OutputFormat::Human);
        assert!(out.contains("a.md"));
        assert!(out.contains("b.md"));
        assert!(out.contains("2 error(s) in 2 file(s)"));
    }

    #[test]
    fn human_empty() {
        let out = format_diagnostics(&[], OutputFormat::Human);
        assert_eq!(out, "All files valid.");
    }

    #[test]
    fn json_single_error() {
        let diagnostics = vec![diag("test.md", "title", DiagnosticKind::MissingRequired)];
        let out = format_diagnostics(&diagnostics, OutputFormat::Json);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&out).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["file"], "test.md");
        assert_eq!(parsed[0]["field"], "title");
        assert!(parsed[0]["message"].as_str().unwrap().contains("required"));
    }

    #[test]
    fn json_empty() {
        let out = format_diagnostics(&[], OutputFormat::Json);
        assert_eq!(out, "[]");
    }

    #[test]
    fn github_single_error() {
        let diagnostics = vec![diag("test.md", "title", DiagnosticKind::MissingRequired)];
        let out = format_diagnostics(&diagnostics, OutputFormat::Github);
        assert!(out.starts_with("::error file=test.md"));
        assert!(out.contains("title"));
        assert!(out.contains("required field missing"));
    }

    #[test]
    fn github_multiple_errors() {
        let diagnostics = vec![
            diag("a.md", "title", DiagnosticKind::MissingRequired),
            diag("b.md", "date", DiagnosticKind::MissingRequired),
        ];
        let out = format_diagnostics(&diagnostics, OutputFormat::Github);
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("::error file=a.md"));
        assert!(lines[1].starts_with("::error file=b.md"));
    }
}
