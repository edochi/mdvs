//! Search command outcome types.

use serde::Serialize;

use crate::block::{Block, Render, TableStyle};
use crate::index::backend::SearchHit;

/// Full outcome for the search command.
#[derive(Debug, Serialize)]
pub struct SearchOutcome {
    /// The query string.
    pub query: String,
    /// Files ranked by cosine similarity, descending.
    pub hits: Vec<SearchHit>,
    /// Name of the embedding model used.
    pub model_name: String,
    /// Result limit that was applied.
    pub limit: usize,
}

impl Render for SearchOutcome {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        // Summary line
        let hit_word = if self.hits.len() == 1 { "hit" } else { "hits" };
        blocks.push(Block::Line(format!(
            "Searched \"{}\" — {} {hit_word}",
            self.query,
            self.hits.len()
        )));
        blocks.push(Block::Line(String::new()));

        // Top-level fields
        blocks.push(Block::Table {
            headers: None,
            rows: vec![
                vec!["query".into(), self.query.clone()],
                vec!["model".into(), self.model_name.clone()],
                vec!["limit".into(), self.limit.to_string()],
            ],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        });

        // Per-hit KeyValue tables
        for (i, hit) in self.hits.iter().enumerate() {
            let mut rows = vec![
                vec!["file".into(), hit.filename.clone()],
                vec!["score".into(), format!("{:.3}", hit.score)],
            ];

            if let (Some(start), Some(end)) = (hit.start_line, hit.end_line) {
                rows.push(vec!["lines".into(), format!("{start}-{end}")]);
            }

            if let Some(ref text) = hit.chunk_text {
                rows.push(vec!["text".into(), text.trim().to_string()]);
            }

            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::KeyValue {
                    title: format!("#{}", i + 1),
                },
            });
        }

        blocks
    }
}
