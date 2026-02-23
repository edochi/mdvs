use std::collections::HashMap;

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
