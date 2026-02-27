mod diagnostic;
mod output;
mod validate;

pub use diagnostic::{Diagnostic, DiagnosticKind};
pub use output::{OutputFormat, format_diagnostics};
pub use validate::validate;
