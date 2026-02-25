use pulldown_cmark::{Event, Parser, Tag, TagEnd};
use text_splitter::MarkdownSplitter;

/// A chunk of a markdown file's body, ready for embedding.
#[derive(Debug, Clone)]
pub struct Chunk {
    pub chunk_index: usize,
    pub start_line: usize, // 1-based
    pub end_line: usize,   // 1-based
    pub plain_text: String, // for embedding (not stored in Parquet)
}

/// Chunk a markdown body into pieces with line offsets and extracted plain text.
pub fn chunk_body(body: &str, max_chars: usize) -> Vec<Chunk> {
    let splitter = MarkdownSplitter::new(max_chars);
    let chunks: Vec<&str> = splitter.chunks(body).collect();

    // Pre-compute line start byte offsets for O(1) line lookups
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(body.match_indices('\n').map(|(i, _)| i + 1))
        .collect();

    chunks
        .iter()
        .enumerate()
        .map(|(i, chunk_md)| {
            // Find byte offset of this chunk in the original body
            let byte_offset = find_chunk_offset(body, chunk_md);
            let chunk_end_byte = byte_offset + chunk_md.len();

            let start_line = byte_offset_to_line(&line_starts, byte_offset);
            let end_line = byte_offset_to_line(&line_starts, chunk_end_byte.saturating_sub(1));

            let plain_text = extract_plain_text(chunk_md);

            Chunk {
                chunk_index: i,
                start_line,
                end_line,
                plain_text,
            }
        })
        .collect()
}

/// Find the byte offset of a chunk slice within the original body.
///
/// `MarkdownSplitter::chunks()` returns subslices of the input, so we can use
/// pointer arithmetic to find the offset.
fn find_chunk_offset(body: &str, chunk: &str) -> usize {
    let body_start = body.as_ptr() as usize;
    let chunk_start = chunk.as_ptr() as usize;
    chunk_start - body_start
}

/// Convert a byte offset to a 1-based line number.
fn byte_offset_to_line(line_starts: &[usize], byte_offset: usize) -> usize {
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i + 1,
        Err(i) => i, // i is the line this byte falls on (0-indexed), +1 for 1-based but Err already gives us the right value
    }
}

/// Extract plain text from markdown using only `Event::Text` events.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_body_single_chunk() {
        let body = "Hello world";
        let chunks = chunk_body(body, 1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 1);
        assert_eq!(chunks[0].plain_text, "Hello world");
    }

    #[test]
    fn chunk_body_line_offsets() {
        let body = "# Heading\n\nParagraph one.\n\n# Second\n\nParagraph two.";
        let chunks = chunk_body(body, 20);
        assert!(chunks.len() >= 2);
        // First chunk starts at line 1
        assert_eq!(chunks[0].start_line, 1);
        // Last chunk ends at the last line
        let last = chunks.last().unwrap();
        assert!(last.end_line >= 5);
    }

    #[test]
    fn extract_plain_text_strips_markdown() {
        let md = "# Hello\n\nThis is **bold** and *italic*.";
        let text = extract_plain_text(md);
        assert!(text.contains("Hello"));
        assert!(text.contains("bold"));
        assert!(!text.contains("**"));
    }

    #[test]
    fn extract_heading_finds_first() {
        let md = "# First\n\n## Second\n\nContent";
        assert_eq!(extract_heading(md), Some("First".to_string()));
    }

    #[test]
    fn extract_heading_none_when_missing() {
        let md = "Just some text without headings.";
        assert_eq!(extract_heading(md), None);
    }
}
