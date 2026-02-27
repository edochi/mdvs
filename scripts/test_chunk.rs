#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! text-splitter = { version = "0.18", features = ["markdown"] }
//! pulldown-cmark = "0.12"
//! regex = "1"
//! ```

use pulldown_cmark::{Event, Parser};
use regex::Regex;
use text_splitter::MarkdownSplitter;

// ============================================================================
// Chunk / Chunks
// ============================================================================

#[derive(Debug, Clone)]
struct Chunk {
    chunk_index: usize,
    start_line: usize, // 1-based
    end_line: usize,   // 1-based
    plain_text: String, // for embedding (not stored in Parquet)
}

struct Chunks(Vec<Chunk>);

impl std::ops::Deref for Chunks {
    type Target = Vec<Chunk>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Chunks {
    fn new(body: &str, max_chars: usize) -> Self {
        let splitter = MarkdownSplitter::new(max_chars);
        let chunks: Vec<&str> = splitter.chunks(body).collect();

        // Pre-compute line start byte offsets for O(1) line lookups
        let line_starts: Vec<usize> = std::iter::once(0)
            .chain(body.match_indices('\n').map(|(i, _)| i + 1))
            .collect();

        let inner = chunks
            .iter()
            .enumerate()
            .map(|(i, chunk_md)| {
                let byte_offset = chunk_byte_offset(body, chunk_md);
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

/// MarkdownSplitter returns subslices of the input, so pointer arithmetic
/// gives us the byte offset within the original body.
fn chunk_byte_offset(body: &str, chunk: &str) -> usize {
    let body_start = body.as_ptr() as usize;
    let chunk_start = chunk.as_ptr() as usize;
    chunk_start - body_start
}

/// Binary search over pre-computed line start offsets → 1-based line number.
fn byte_offset_to_line(line_starts: &[usize], byte_offset: usize) -> usize {
    match line_starts.binary_search(&byte_offset) {
        Ok(i) => i + 1,
        Err(i) => i,
    }
}

// ============================================================================
// Plain text extraction
// ============================================================================

/// Extract plain text from markdown:
/// 1. pulldown-cmark strips standard markdown formatting
/// 2. Regex strips Obsidian wikilinks: [[target]], [[target|display]], ![[embed]]
fn extract_plain_text(markdown: &str) -> String {
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
fn strip_wikilinks(text: &str) -> String {
    let re = Regex::new(r"!?\[\[([^\]|]+)(?:\|([^\]]+))?\]\]").unwrap();
    re.replace_all(text, |caps: &regex::Captures| {
        caps.get(2)
            .unwrap_or_else(|| caps.get(1).unwrap())
            .as_str()
            .to_string()
    })
    .into_owned()
}

// ============================================================================
// Tests
// ============================================================================

fn main() {
    println!("=== Chunking tests ===\n");

    // --- Test 1: Single chunk ---
    {
        let body = "Hello world";
        let chunks = Chunks::new(body,1000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_index, 0);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 1);
        assert_eq!(chunks[0].plain_text, "Hello world");
        println!("  1. Single chunk  ✓");
    }

    // --- Test 2: Empty body ---
    {
        let chunks = Chunks::new("", 1000);
        assert!(chunks.is_empty());
        println!("  2. Empty body  ✓");
    }

    // --- Test 3: Multi-chunk with line offsets ---
    {
        let body = "# First\n\nLine two.\n\n# Second\n\nLine four.";
        let chunks = Chunks::new(body,25);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 3);
        assert_eq!(chunks[1].start_line, 5);
        assert_eq!(chunks[1].end_line, 7);
        println!("  3. Multi-chunk line offsets  ✓");
    }

    // --- Test 4: Sequential chunk indices ---
    {
        let body = "# A\n\nText.\n\n# B\n\nText.\n\n# C\n\nText.";
        let chunks = Chunks::new(body,15);
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index, i);
        }
        println!("  4. Sequential chunk indices  ✓");
    }

    // --- Test 5: Contiguous lines (no gaps between chunks) ---
    {
        let body = "# A\n\nText one.\n\n# B\n\nText two.\n\n# C\n\nText three.";
        let chunks = Chunks::new(body,20);
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
        println!("  5. Contiguous lines  ✓");
    }

    // --- Test 6: UTF-8 content ---
    {
        let body = "# Café\n\nLa résumé.\n\n# 日本語\n\nテキスト。";
        let chunks = Chunks::new(body,25);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].start_line, 1);
        let last = chunks.last().unwrap();
        assert_eq!(last.end_line, 7);
        println!("  6. UTF-8 content  ✓");
    }

    // --- Test 7: Plain text strips markdown ---
    {
        let text = extract_plain_text("# Hello\n\nThis is **bold** and *italic*.");
        assert!(text.contains("Hello"));
        assert!(text.contains("bold"));
        assert!(text.contains("italic"));
        assert!(!text.contains("**"));
        assert!(!text.contains("#"));
        println!("  7. Plain text strips markdown  ✓");
    }

    // --- Test 8: Plain text strips links ---
    {
        let text = extract_plain_text("Click [here](https://example.com) for more.");
        assert!(text.contains("here"));
        assert!(text.contains("for more."));
        assert!(!text.contains("https://example.com"));
        println!("  8. Plain text strips links  ✓");
    }

    // --- Test 9: Plain text preserves code block content ---
    {
        let text = extract_plain_text("Before.\n\n```rust\nlet x = 1;\n```\n\nAfter.");
        assert!(text.contains("Before."));
        assert!(text.contains("After."));
        assert!(text.contains("let x = 1;"));
        println!("  9. Plain text preserves code block content  ✓");
    }

    // --- Test 10: Wikilink [[target]] → target ---
    {
        let text = strip_wikilinks("See [[some note]] for details.");
        assert_eq!(text, "See some note for details.");
        println!("  10. [[target]] → target  ✓");
    }

    // --- Test 11: Wikilink [[target|display]] → display ---
    {
        let text = strip_wikilinks("See [[some note|my note]] for details.");
        assert_eq!(text, "See my note for details.");
        println!("  11. [[target|display]] → display  ✓");
    }

    // --- Test 12: Embed ![[file]] → file ---
    {
        let text = strip_wikilinks("Here is ![[diagram.png]] inline.");
        assert_eq!(text, "Here is diagram.png inline.");
        println!("  12. ![[file]] → file  ✓");
    }

    // --- Test 13: Embed ![[file|alt]] → alt ---
    {
        let text = strip_wikilinks("Here is ![[diagram.png|my diagram]] inline.");
        assert_eq!(text, "Here is my diagram inline.");
        println!("  13. ![[file|alt]] → alt  ✓");
    }

    // --- Test 14: Multiple wikilinks in one string ---
    {
        let text = strip_wikilinks("Link to [[A]] and [[B|Beta]] and ![[C]].");
        assert_eq!(text, "Link to A and Beta and C.");
        println!("  14. Multiple wikilinks  ✓");
    }

    // --- Test 15: No wikilinks → unchanged ---
    {
        let text = strip_wikilinks("Just normal text.");
        assert_eq!(text, "Just normal text.");
        println!("  15. No wikilinks → unchanged  ✓");
    }

    // --- Test 16: Wikilinks inside markdown (full pipeline) ---
    {
        let text = extract_plain_text("# Notes\n\nSee [[other note|linked]] for **details**.");
        assert!(text.contains("Notes"));
        assert!(text.contains("linked"));
        assert!(text.contains("details"));
        assert!(!text.contains("[["));
        assert!(!text.contains("**"));
        println!("  16. Wikilinks inside markdown (full pipeline)  ✓");
    }

    // --- Test 17: Plain text empty input ---
    {
        let text = extract_plain_text("");
        assert_eq!(text, "");
        println!("  17. Empty input  ✓");
    }

    println!("\n=== All tests passed ===");
}
