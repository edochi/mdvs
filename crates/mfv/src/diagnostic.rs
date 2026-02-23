use std::fmt;

/// The kind of validation problem found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// A required field is missing.
    MissingRequired,
    /// Field value has the wrong type (expected, got).
    WrongType { expected: String, got: String },
    /// Field value doesn't match the required pattern.
    PatternMismatch { pattern: String, value: String },
    /// Field value is not in the allowed enum values.
    InvalidEnum { value: String, allowed: Vec<String> },
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticKind::MissingRequired => write!(f, "required field missing"),
            DiagnosticKind::WrongType { expected, got } => {
                write!(f, "expected type '{expected}', got '{got}'")
            }
            DiagnosticKind::PatternMismatch { pattern, value } => {
                write!(f, "value \"{value}\" does not match pattern /{pattern}/")
            }
            DiagnosticKind::InvalidEnum { value, allowed } => {
                write!(
                    f,
                    "value \"{value}\" not in allowed values [{}]",
                    allowed.join(", ")
                )
            }
        }
    }
}

/// A single validation diagnostic.
#[derive(Debug, Clone)]
pub struct Diagnostic {
    /// Relative path of the file.
    pub file: String,
    /// Field name that has the problem.
    pub field: String,
    /// What's wrong.
    pub kind: DiagnosticKind,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: field '{}': {}", self.file, self.field, self.kind)
    }
}
