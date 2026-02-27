use std::fmt;

/// The kind of validation problem found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    /// A required field is missing.
    MissingRequired,
    /// Field value has the wrong type.
    WrongType {
        /// Type name from the schema.
        expected: String,
        /// Type name inferred from the actual value.
        got: String,
    },
    /// Field value doesn't match the required pattern.
    PatternMismatch {
        /// Regex pattern from the schema.
        pattern: String,
        /// Actual value that failed to match.
        value: String,
    },
    /// Field value is not in the allowed enum values.
    InvalidEnum {
        /// Actual value found.
        value: String,
        /// List of allowed values from the schema.
        allowed: Vec<String>,
    },
    /// Field is present but not allowed at this file's path.
    NotAllowed,
    /// Date value doesn't match the expected date format.
    DateFormatMismatch {
        /// Expected chrono format string from the schema.
        format: String,
        /// Actual value that failed to match.
        value: String,
    },
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
            DiagnosticKind::NotAllowed => write!(f, "field not allowed here"),
            DiagnosticKind::DateFormatMismatch { format, value } => {
                write!(f, "value \"{value}\" does not match date format '{format}'")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_missing_required() {
        let kind = DiagnosticKind::MissingRequired;
        assert_eq!(kind.to_string(), "required field missing");
    }

    #[test]
    fn display_wrong_type() {
        let kind = DiagnosticKind::WrongType {
            expected: "string".to_string(),
            got: "integer".to_string(),
        };
        assert_eq!(kind.to_string(), "expected type 'string', got 'integer'");
    }

    #[test]
    fn display_pattern_mismatch() {
        let kind = DiagnosticKind::PatternMismatch {
            pattern: r"^\d{4}-\d{2}-\d{2}$".to_string(),
            value: "not-a-date".to_string(),
        };
        let s = kind.to_string();
        assert!(s.contains(r"^\d{4}-\d{2}-\d{2}$"));
        assert!(s.contains("not-a-date"));
    }

    #[test]
    fn display_not_allowed() {
        let kind = DiagnosticKind::NotAllowed;
        assert_eq!(kind.to_string(), "field not allowed here");
    }

    #[test]
    fn display_invalid_enum() {
        let kind = DiagnosticKind::InvalidEnum {
            value: "archived".to_string(),
            allowed: vec!["draft".to_string(), "published".to_string()],
        };
        let s = kind.to_string();
        assert!(s.contains("archived"));
        assert!(s.contains("draft, published"));
    }
}
