mod discovery;
pub mod field_def;
mod field_type;
pub mod lock;
mod schema;

pub use discovery::{FieldInfo, auto_promote, discover_fields, infer_type, is_date_string};
pub use field_def::FieldDef;
pub use field_type::FieldType;
pub use lock::LockFile;
pub use schema::{Schema, SchemaError};
