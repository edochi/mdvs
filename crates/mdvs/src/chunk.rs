use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use text_splitter::MarkdownSplitter;

use crate::types::ChunkData;

/// Chunk a note's body into pieces, extracting plain text and headings for each.
pub fn chunk_note(filename: &str, body: &str, max_chars: usize) -> Vec<ChunkData> {
    let splitter = MarkdownSplitter::new(max_chars);
    let chunks: Vec<&str> = splitter.chunks(body).collect();

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk_md)| {
            let plain_text = extract_plain_text(chunk_md);
            let heading = extract_heading(chunk_md);
            let char_count = plain_text.chars().count();
            let chunk_id = format!("{filename}#{i}");

            ChunkData {
                chunk_id,
                filename: filename.to_string(),
                chunk_index: i,
                heading,
                plain_text,
                char_count,
            }
        })
        .collect()
}

/// Extract plain text from markdown using only `Event::Text` events (v0.1).
pub fn extract_plain_text(markdown: &str) -> String {
    let parser = Parser::new(markdown);
    let mut text = String::new();

    for event in parser {
        if let Event::Text(t) = event {
            text.push_str(&t);
            text.push(' ');
        }
    }

    text.trim().to_string()
}

/// Extract the first heading from a markdown chunk.
pub fn extract_heading(markdown: &str) -> Option<String> {
    let parser = Parser::new(markdown);
    let mut in_heading = false;
    let mut heading_text = String::new();

    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading_text.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                if in_heading && !heading_text.is_empty() {
                    return Some(heading_text.trim().to_string());
                }
                in_heading = false;
            }
            Event::Text(t) if in_heading => {
                heading_text.push_str(&t);
            }
            _ => {}
        }
    }

    None
}
