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

        let hit_word = if self.hits.len() == 1 { "hit" } else { "hits" };
        blocks.push(Block::Line(format!(
            "Searched \"{}\" — {} {hit_word}",
            self.query,
            self.hits.len()
        )));

        if self.hits.is_empty() {
            return blocks;
        }

        // Per-hit record tables with chunk text
        for (i, hit) in self.hits.iter().enumerate() {
            let idx = format!("{}", i + 1);
            let path = format!("\"{}\"", hit.filename);
            let score = format!("{:.3}", hit.score);

            let detail = match (&hit.chunk_text, hit.start_line, hit.end_line) {
                (Some(text), Some(start), Some(end)) => {
                    let indented: String = text
                        .lines()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("  lines {start}-{end}:\n{indented}")
                }
                (None, Some(start), Some(end)) => format!("  lines {start}-{end}"),
                _ => String::new(),
            };

            let mut rows = vec![vec![idx, path, score]];
            if !detail.is_empty() {
                rows.push(vec![detail, String::new(), String::new()]);
            }

            blocks.push(Block::Table {
                headers: None,
                rows: rows.clone(),
                style: if rows.len() > 1 {
                    TableStyle::Record {
                        detail_rows: vec![1],
                    }
                } else {
                    TableStyle::Compact
                },
            });
        }

        // Footer
        blocks.push(Block::Line(format!(
            "{} {hit_word} | model: \"{}\" | limit: {}",
            self.hits.len(),
            self.model_name,
            self.limit,
        )));

        blocks
    }
}

/// Compact search hit — filename and score only.
#[derive(Debug, Serialize)]
pub struct SearchHitCompact {
    /// Filename of the matched file.
    pub filename: String,
    /// Cosine similarity score.
    pub score: f64,
}

impl From<&SearchHit> for SearchHitCompact {
    fn from(h: &SearchHit) -> Self {
        Self {
            filename: h.filename.clone(),
            score: h.score,
        }
    }
}

/// Compact outcome for the search command.
#[derive(Debug, Serialize)]
pub struct SearchOutcomeCompact {
    /// The query string.
    pub query: String,
    /// Compact hits (filename + score only).
    pub hits: Vec<SearchHitCompact>,
    /// Name of the embedding model used.
    pub model_name: String,
    /// Result limit that was applied.
    pub limit: usize,
}

impl Render for SearchOutcomeCompact {
    fn render(&self) -> Vec<Block> {
        let mut blocks = vec![];

        let hit_word = if self.hits.len() == 1 { "hit" } else { "hits" };
        blocks.push(Block::Line(format!(
            "Searched \"{}\" — {} {hit_word}",
            self.query,
            self.hits.len()
        )));

        if !self.hits.is_empty() {
            let rows: Vec<Vec<String>> = self
                .hits
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    vec![
                        format!("{}", i + 1),
                        format!("\"{}\"", h.filename),
                        format!("{:.3}", h.score),
                    ]
                })
                .collect();
            blocks.push(Block::Table {
                headers: None,
                rows,
                style: TableStyle::Compact,
            });
        }

        blocks
    }
}

impl From<&SearchOutcome> for SearchOutcomeCompact {
    fn from(o: &SearchOutcome) -> Self {
        Self {
            query: o.query.clone(),
            hits: o.hits.iter().map(SearchHitCompact::from).collect(),
            model_name: o.model_name.clone(),
            limit: o.limit,
        }
    }
}
