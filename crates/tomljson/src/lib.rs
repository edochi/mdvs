// Regression gate for TODO-0180: catch new panic-emitters in non-test code
// at PR time. Tests use `.unwrap()` / `.expect()` liberally and are
// excluded via `cfg(not(test))`. Surviving cases in production code must
// carry a local `#[allow(...)]` with a justifying comment.
#![cfg_attr(
    not(test),
    warn(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

//! Lossless TOML ↔ JSON translation.
//!
//! `tomljson` translates `serde_json::Value` to TOML and back, handling the
//! representational gaps where TOML can't express something JSON can:
//!
//! - **Null**: TOML has no null type. JSON `null` is encoded as a string
//!   placeholder (default `"__null__"`, configurable via [`TomlJsonOptions`]).
//!   Decoding substitutes the placeholder back to JSON `null`.
//! - **Top-level non-table values**: TOML documents must be a table at the
//!   root. Non-table JSON values (booleans, scalars, arrays) are wrapped under
//!   a reserved `__root__` key on encode and unwrapped on decode.
//! - **Integer range**: TOML integers are signed 64-bit. Encoding refuses
//!   values larger than `i64::MAX` (per TOML's spec, not a tomljson limitation).
//!
//! The motivating use case is JSON Schema 2020-12 documents authored as TOML.
//! `tomljson` is mdvs-agnostic — anyone moving JSON-shaped data through TOML
//! can use it.
//!
//! # Quick start
//!
//! ```
//! use serde_json::json;
//!
//! let value = json!({
//!     "type": "object",
//!     "properties": {
//!         "name": { "type": "string" }
//!     }
//! });
//!
//! let toml_str = tomljson::to_string(&value).unwrap();
//! let back = tomljson::from_str(&toml_str).unwrap();
//! assert_eq!(back, value);
//! ```

mod de;
mod error;
mod ser;

pub use de::from_str_with_options;
pub use error::{Error, Result};
pub use ser::to_string_with_options;

/// Default placeholder string for JSON `null` values.
pub const DEFAULT_NULL_PLACEHOLDER: &str = "__null__";

/// Default key used to wrap top-level non-table JSON values (booleans, scalars,
/// arrays) in TOML output, since TOML documents must have a table at the root.
pub const DEFAULT_ROOT_PLACEHOLDER: &str = "__root__";

/// Options for encode and decode operations.
#[derive(Debug, Clone)]
pub struct TomlJsonOptions {
    /// String used to represent JSON `null` values in TOML output. Default:
    /// `"__null__"`. If any string in the input matches this placeholder, the
    /// encoder errors with [`Error::PlaceholderCollision`] — pick a different
    /// placeholder unique to your data.
    pub null_placeholder: String,

    /// Key used to wrap top-level non-table JSON values (booleans, scalars,
    /// arrays). The encoder emits `<root_placeholder> = <value>` for non-table
    /// roots; the decoder unwraps a single-key table whose only key matches
    /// this. Default: `"__root__"`. If the input is an Object whose top-level
    /// keys include this name, the encoder errors with
    /// [`Error::RootKeyCollision`] — pick a different name unique to your data.
    pub root_placeholder: String,
}

impl Default for TomlJsonOptions {
    fn default() -> Self {
        Self {
            null_placeholder: DEFAULT_NULL_PLACEHOLDER.to_string(),
            root_placeholder: DEFAULT_ROOT_PLACEHOLDER.to_string(),
        }
    }
}

/// Encode a JSON value to TOML using default options.
///
/// Convenience wrapper over [`to_string_with_options`].
pub fn to_string(value: &serde_json::Value) -> Result<String> {
    to_string_with_options(value, &TomlJsonOptions::default())
}

/// Decode a TOML string to a JSON value using default options.
///
/// Convenience wrapper over [`from_str_with_options`].
pub fn from_str(s: &str) -> Result<serde_json::Value> {
    from_str_with_options(s, &TomlJsonOptions::default())
}
