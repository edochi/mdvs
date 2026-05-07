use std::fmt;

/// Errors produced by `tomljson`'s encode and decode paths.
#[derive(Debug)]
pub enum Error {
    /// A string in the input matches the configured null placeholder, which
    /// would round-trip ambiguously to `null` on decode. Pick a different
    /// placeholder via `TomlJsonOptions::null_placeholder`.
    PlaceholderCollision { path: String, placeholder: String },

    /// A JSON unsigned integer exceeds `i64::MAX`. TOML integers are signed
    /// 64-bit; the value cannot be represented losslessly.
    IntegerOutOfRange { path: String, value: u64 },

    /// A TOML float (`NaN`, `+inf`, or `-inf`) cannot be represented in JSON.
    /// `serde_json::Number::from_f64` rejects these; JSON Schema's validation
    /// model has no notion of them either. Producers should omit
    /// `maximum`/`minimum` to indicate "no bound" rather than encoding
    /// infinities.
    FloatNotRepresentable { path: String, kind: &'static str },

    /// Failure to parse the input as TOML.
    Toml(toml::de::Error),

    /// I/O error from the underlying writer.
    Io(std::io::Error),

    /// Formatting error from `std::fmt`.
    Fmt(fmt::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::PlaceholderCollision { path, placeholder } => {
                write!(
                    f,
                    "string at {path:?} equals the null placeholder {placeholder:?}; \
                     pick a different placeholder via TomlJsonOptions"
                )
            }
            Error::IntegerOutOfRange { path, value } => {
                write!(
                    f,
                    "integer {value} at {path:?} exceeds TOML's signed 64-bit range \
                     (i64::MAX = 9223372036854775807)"
                )
            }
            Error::FloatNotRepresentable { path, kind } => {
                write!(
                    f,
                    "TOML float {kind} at {path:?} cannot be represented in JSON; \
                     omit the value or use a sentinel string instead"
                )
            }
            Error::Toml(e) => write!(f, "toml parse error: {e}"),
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::Fmt(e) => write!(f, "format error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Toml(e) => Some(e),
            Error::Io(e) => Some(e),
            Error::Fmt(e) => Some(e),
            _ => None,
        }
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::Toml(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<fmt::Error> for Error {
    fn from(e: fmt::Error) -> Self {
        Error::Fmt(e)
    }
}

/// Convenience alias for `Result<T, tomljson::Error>`.
pub type Result<T> = std::result::Result<T, Error>;
