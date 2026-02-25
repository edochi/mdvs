//! Field definitions, type system, TOML schema parsing, and frontmatter field discovery for mdvs.
#![warn(missing_docs)]

mod discovery;
/// Field definition types and TOML deserialization.
pub mod field_def;
mod field_type;
mod inference;
/// Lock file types for capturing discovery snapshots.
pub mod lock;
mod schema;

pub use discovery::{FieldInfo, discover_fields, infer_type, is_date_string};
pub use field_def::FieldDef;
pub use field_type::FieldType;
pub use inference::{FieldPaths, infer_field_paths};
pub use lock::{LockError, LockFile};
pub use schema::{FrontmatterFormat, Schema, SchemaError};
