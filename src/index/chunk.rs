use pulldown_cmark::{Event, Parser};
use regex::Regex;
use text_splitter::MarkdownSplitter;

#[derive(Debug, Clone)]
pub struct Chunk {
    pub chunk_index: usize,
    pub start_line: usize, // 1-based
    pub end_line: usize,   // 1-based
    pub plain_text: String,
}

pub struct Chunks(Vec<Chunk>);

impl std::ops::Deref for Chunks {
    type Target = Vec<Chunk>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Chunks {
    pub fn new(body: &str, max_chars: usize) -> Self {
        let splitter = MarkdownSplitter::new(max_chars);

        // Pre-compute line start byte offsets for O(1) line lookups
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(body.match_indices('\n').map(|(i, _)| i + 1))
            .collect();

        let inner = splitter
            .chunk_indices(body)
            .enumerate()
            .map(|(i, (byte_offset, chunk_md))| {
                let chunk_end_byte = byte_offset + chunk_md.len();

                let start_line = byte_offset_to_line(&line_starts, byte_offset);
                let end_line =
                    byte_offset_to_line(&line_starts, chunk_end_byte.saturating_sub(1));

                let plain_text = extract_plain_text(chunk_md);

                Chunk {
                    chunk_index: i,
                    start_line,
                    end_line,
                    plain_text,
                }
            })
            .collect();

        Chunks(inner)
    }
}

/// Binary search over pre-computed line start offsets → 1-based line number.
fn byte_offset_to_line(line_starts: &[usize], byte_offset: usize) -> usize {
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i + 1,
        Err(i) => i,
    }
}

/// Extract plain text from markdown:
/// 1. pulldown-cmark strips standard markdown formatting
/// 2. Regex strips Obsidian wikilinks: [[target]], [[target|display]], ![[embed]]
pub fn extract_plain_text(markdown: &str) -> String {
    let parser = Parser::new(markdown);
    let mut text = String::new();

    for event in parser {
        if let Event::Text(t) = event {
            text.push_str(&t);
            text.push(' ');
        }
    }

    strip_wikilinks(text.trim())
}

/// Replace wikilinks with their display text:
/// - [[target]] → target
/// - [[target|display]] → display
/// - ![[embed]] → embed
/// - ![[embed|display]] → display
pub fn strip_wikilinks(text: &str) -> String {
    let re = Regex::new(r"!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").unwrap();
    re.replace_all(text, |caps: &regex::Captures| {
        caps.get(2)
            .unwrap_or_else(|| caps.get(1).unwrap())
            .as_str()
            .to_string()
    })
    .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_chunk() {
        let chunks = Chunks::new("Hello world", 1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 1);
        assert_eq!(chunks[0].plain_text, "Hello world");
    }

    #[test]
    fn empty_body() {
        let chunks = Chunks::new("", 1000);
        assert!(chunks.is_empty());
    }

    #[test]
    fn multi_chunk_line_offsets() {
        let body = "# First\n\nLine two.\n\n# Second\n\nLine four.";
        let chunks = Chunks::new(body, 25);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[1].start_line, 5);
        assert_eq!(chunks[1].end_line, 7);
    }

    #[test]
    fn sequential_chunk_indices() {
        let body = "# A\n\nText.\n\n# B\n\nText.\n\n# C\n\nText.";
        let chunks = Chunks::new(body, 15);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
        }
    }

    #[test]
    fn contiguous_lines() {
        let body = "# A\n\nText one.\n\n# B\n\nText two.\n\n# C\n\nText three.";
        let chunks = Chunks::new(body, 20);
        assert!(chunks.len() >= 2);
        for pair in chunks.windows(2) {
            assert!(
                pair[1].start_line > pair[0].end_line,
                "chunk {} ends at line {} but chunk {} starts at line {}",
                pair[0].chunk_index,
                pair[0].end_line,
                pair[1].chunk_index,
                pair[1].start_line,
            );
        }
    }

    #[test]
    fn utf8_content() {
        let body = "# Café\n\nLa résumé.\n\n# 日本語\n\nテキスト。";
        let chunks = Chunks::new(body, 25);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].start_line, 1);
        let last = chunks.last().unwrap();
        assert_eq!(last.end_line, 7);
    }

    #[test]
    fn plain_text_strips_markdown() {
        let text = extract_plain_text("# Hello\n\nThis is **bold** and *italic*.");
        assert!(text.contains("Hello"));
        assert!(text.contains("bold"));
        assert!(text.contains("italic"));
        assert!(!text.contains("**"));
        assert!(!text.contains("#"));
    }

    #[test]
    fn plain_text_strips_links() {
        let text = extract_plain_text("Click [here](https://example.com) for more.");
        assert!(text.contains("here"));
        assert!(text.contains("for more."));
        assert!(!text.contains("https://example.com"));
    }

    #[test]
    fn plain_text_preserves_code_block_content() {
        let text = extract_plain_text("Before.\n\n```rust\nlet x = 1;\n```\n\nAfter.");
        assert!(text.contains("Before."));
        assert!(text.contains("After."));
        assert!(text.contains("let x = 1;"));
    }

    #[test]
    fn wikilink_target() {
        let text = strip_wikilinks("See [[some note]] for details.");
        assert_eq!(text, "See some note for details.");
    }

    #[test]
    fn wikilink_display() {
        let text = strip_wikilinks("See [[some note|my note]] for details.");
        assert_eq!(text, "See my note for details.");
    }

    #[test]
    fn embed_file() {
        let text = strip_wikilinks("Here is ![[diagram.png]] inline.");
        assert_eq!(text, "Here is diagram.png inline.");
    }

    #[test]
    fn embed_alt() {
        let text = strip_wikilinks("Here is ![[diagram.png|my diagram]] inline.");
        assert_eq!(text, "Here is my diagram inline.");
    }

    #[test]
    fn multiple_wikilinks() {
        let text = strip_wikilinks("Link to [[A]] and [[B|Beta]] and ![[C]].");
        assert_eq!(text, "Link to A and Beta and C.");
    }

    #[test]
    fn no_wikilinks_unchanged() {
        let text = strip_wikilinks("Just normal text.");
        assert_eq!(text, "Just normal text.");
    }

    #[test]
    fn wikilinks_inside_markdown() {
        let text = extract_plain_text("# Notes\n\nSee [[other note|linked]] for **details**.");
        assert!(text.contains("Notes"));
        assert!(text.contains("linked"));
        assert!(text.contains("details"));
        assert!(!text.contains("[["));
        assert!(!text.contains("**"));
    }

    #[test]
    fn plain_text_empty_input() {
        let text = extract_plain_text("");
        assert_eq!(text, "");
    }
}
