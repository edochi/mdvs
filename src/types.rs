use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldType {
    String,
    StringArray,
    Date,
    Boolean,
    Integer,
    Float,
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::String => write!(f, "String"),
            FieldType::StringArray => write!(f, "String[]"),
            FieldType::Date => write!(f, "Date"),
            FieldType::Boolean => write!(f, "Boolean"),
            FieldType::Integer => write!(f, "Integer"),
            FieldType::Float => write!(f, "Float"),
        }
    }
}

impl FieldType {
    pub fn sql_type(&self) -> &'static str {
        match self {
            FieldType::String => "VARCHAR",
            FieldType::StringArray => "VARCHAR[]",
            FieldType::Date => "DATE",
            FieldType::Boolean => "BOOLEAN",
            FieldType::Integer => "BIGINT",
            FieldType::Float => "DOUBLE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub field_type: FieldType,
    pub count: usize,
    pub promoted: bool,
}

#[derive(Debug, Clone)]
pub struct NoteData {
    pub filename: String,
    pub frontmatter: Option<serde_json::Value>,
    pub body: String,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub struct ChunkData {
    pub chunk_id: String,
    pub filename: String,
    pub chunk_index: usize,
    pub heading: Option<String>,
    pub plain_text: String,
    pub char_count: usize,
}

#[derive(Debug)]
pub struct SearchResult {
    pub filename: String,
    pub promoted: HashMap<String, serde_json::Value>,
    pub distance: f64,
    pub best_heading: Option<String>,
    pub snippet: String,
}
