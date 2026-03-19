//! Shared formatters that consume `Vec<Block>` and produce formatted output.
//!
//! Two formatters: `format_text` (terminal, box-drawing tables via tabled)
//! and `format_markdown` (pipe tables, section headers). Adding a new output
//! format means writing one function here — no command code changes needed.

use tabled::settings::{
    object::Cell, peaker::PriorityMax, span::ColumnSpan, style::Style, themes::BorderCorrection,
    width::Width, Modify,
};

use crate::block::{Block, TableStyle};
use crate::table::{style_compact, term_width, Builder};

/// Format blocks as terminal text with box-drawing tables.
pub fn format_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        format_text_block(block, &mut out, 0);
    }
    out
}

fn format_text_block(block: &Block, out: &mut String, indent: usize) {
    let prefix = " ".repeat(indent);
    match block {
        Block::Line(s) => {
            out.push_str(&prefix);
            out.push_str(s);
            out.push('\n');
        }
        Block::Table {
            headers,
            rows,
            style,
        } => {
            let mut builder = Builder::default();
            if let Some(hdrs) = headers {
                builder.push_record(hdrs.iter().map(String::as_str));
            }
            for row in rows {
                builder.push_record(row.iter().map(String::as_str));
            }
            let mut table = builder.build();

            match style {
                TableStyle::Compact => {
                    style_compact(&mut table);
                }
                TableStyle::Record { detail_rows } => {
                    let col_count = headers
                        .as_ref()
                        .map(|h| h.len())
                        .or_else(|| rows.first().map(|r| r.len()))
                        .unwrap_or(1) as isize;
                    let w = term_width();
                    let header_offset = if headers.is_some() { 1 } else { 0 };
                    table.with(Style::rounded());
                    for &row_idx in detail_rows {
                        let actual_row = row_idx + header_offset;
                        table.with(
                            Modify::new(Cell::new(actual_row, 0)).with(ColumnSpan::new(col_count)),
                        );
                    }
                    table.with(BorderCorrection {});
                    table.with(Width::increase(w));
                    table.with(Width::wrap(w).priority(PriorityMax::left()));
                }
            }

            let rendered = table.to_string();
            if indent > 0 {
                for line in rendered.lines() {
                    out.push_str(&prefix);
                    out.push_str(line);
                    out.push('\n');
                }
            } else {
                out.push_str(&rendered);
                out.push('\n');
            }
        }
        Block::Section { label, children } => {
            out.push_str(&prefix);
            out.push_str(label);
            out.push_str(":\n");
            for child in children {
                format_text_block(child, out, indent + 2);
            }
        }
    }
}

/// Format blocks as markdown (pipe tables, section headers).
///
/// Basic implementation — sufficient for initial use. Full markdown formatting
/// is tracked in TODO-0101.
pub fn format_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        format_markdown_block(block, &mut out);
    }
    out
}

fn format_markdown_block(block: &Block, out: &mut String) {
    match block {
        Block::Line(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Block::Table { headers, rows, .. } => {
            if let Some(hdrs) = headers {
                out.push_str("| ");
                out.push_str(&hdrs.join(" | "));
                out.push_str(" |\n");
                out.push_str("| ");
                out.push_str(
                    &hdrs
                        .iter()
                        .map(|_| "---".to_string())
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                out.push_str(" |\n");
            }
            for row in rows {
                out.push_str("| ");
                out.push_str(&row.join(" | "));
                out.push_str(" |\n");
            }
            out.push('\n');
        }
        Block::Section { label, children } => {
            out.push_str("## ");
            out.push_str(label);
            out.push('\n');
            out.push('\n');
            for child in children {
                format_markdown_block(child, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    #[test]
    fn text_empty_blocks() {
        assert_eq!(format_text(&[]), "");
    }

    #[test]
    fn text_line() {
        let blocks = vec![Block::Line("hello world".into())];
        assert_eq!(format_text(&blocks), "hello world\n");
    }

    #[test]
    fn text_multiple_lines() {
        let blocks = vec![Block::Line("line 1".into()), Block::Line("line 2".into())];
        assert_eq!(format_text(&blocks), "line 1\nline 2\n");
    }

    #[test]
    fn text_compact_table() {
        let blocks = vec![Block::Table {
            headers: Some(vec!["name".into(), "type".into()]),
            rows: vec![vec!["title".into(), "String".into()]],
            style: TableStyle::Compact,
        }];
        let output = format_text(&blocks);
        assert!(output.contains("title"));
        assert!(output.contains("String"));
        // Rounded border chars
        assert!(output.contains('╭') || output.contains('│'));
    }

    #[test]
    fn text_record_table() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![
                vec!["\"title\"".into(), "String".into(), "5/5".into()],
                vec![
                    "  required:\n    - \"**\"".into(),
                    String::new(),
                    String::new(),
                ],
            ],
            style: TableStyle::Record {
                detail_rows: vec![1],
            },
        }];
        let output = format_text(&blocks);
        assert!(output.contains("title"));
        assert!(output.contains("required"));
    }

    #[test]
    fn text_section() {
        let blocks = vec![Block::Section {
            label: "Auto-build".into(),
            children: vec![Block::Line("Scan: 5 files".into())],
        }];
        let output = format_text(&blocks);
        assert!(output.contains("Auto-build:"));
        assert!(output.contains("  Scan: 5 files"));
    }

    #[test]
    fn markdown_line() {
        let blocks = vec![Block::Line("hello".into())];
        assert_eq!(format_markdown(&blocks), "hello\n");
    }

    #[test]
    fn markdown_table_with_headers() {
        let blocks = vec![Block::Table {
            headers: Some(vec!["name".into(), "type".into()]),
            rows: vec![vec!["title".into(), "String".into()]],
            style: TableStyle::Compact,
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("| name | type |"));
        assert!(output.contains("| --- | --- |"));
        assert!(output.contains("| title | String |"));
    }

    #[test]
    fn markdown_section() {
        let blocks = vec![Block::Section {
            label: "Results".into(),
            children: vec![Block::Line("5 hits".into())],
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("## Results"));
        assert!(output.contains("5 hits"));
    }
}
