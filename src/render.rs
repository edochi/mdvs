//! Shared formatters that consume `Vec<Block>` and produce formatted output.
//!
//! Two formatters: `format_text` (terminal, box-drawing tables via tabled)
//! and `format_markdown` (pipe tables, section headers). Adding a new output
//! format means writing one function here — no command code changes needed.

use tabled::settings::{
    object::{Column, Rows},
    style::{LineText, Style},
    themes::BorderCorrection,
    width::Width,
    Modify, Panel,
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
            let table = match style {
                TableStyle::Compact => {
                    let mut builder = Builder::default();
                    if let Some(hdrs) = headers {
                        builder.push_record(hdrs.iter().map(String::as_str));
                    }
                    for row in rows {
                        builder.push_record(row.iter().map(String::as_str));
                    }
                    let mut table = builder.build();
                    style_compact(&mut table);
                    table
                }
                TableStyle::Record { detail_rows } => {
                    // Build table with only non-detail rows so detail text
                    // doesn't inflate column widths
                    let mut builder = Builder::default();
                    if let Some(hdrs) = headers {
                        builder.push_record(hdrs.iter().map(String::as_str));
                    }
                    for (i, row) in rows.iter().enumerate() {
                        if !detail_rows.contains(&i) {
                            builder.push_record(row.iter().map(String::as_str));
                        }
                    }
                    let mut table = builder.build();
                    let w = term_width();
                    let header_offset = if headers.is_some() { 1 } else { 0 };
                    table.with(Style::rounded());

                    // Insert detail rows as Panels (spanning rows that don't
                    // affect column width calculation).
                    // Panel::horizontal(n, text) inserts a new row at position n.
                    // We insert after the data row that precedes each detail row.
                    let mut panels_inserted = 0;
                    for &row_idx in detail_rows {
                        let detail_text = &rows[row_idx][0];
                        if !detail_text.is_empty() {
                            // Count non-detail rows before this detail row
                            let data_rows_before =
                                (0..row_idx).filter(|i| !detail_rows.contains(i)).count();
                            // Insert position: after the last data row + header + previously inserted panels
                            let pos = data_rows_before + header_offset + panels_inserted;
                            table.with(Panel::horizontal(pos, detail_text));
                            panels_inserted += 1;
                        }
                    }

                    table.with(BorderCorrection {});
                    // Fixed proportional column widths via per-column Modify
                    let col_count = headers
                        .as_ref()
                        .map(|h| h.len())
                        .or_else(|| rows.first().map(|r| r.len()))
                        .unwrap_or(1);
                    // Overhead: borders (col_count + 1 chars) + padding (2 per col)
                    let overhead = (col_count + 1) + (col_count * 2);
                    let available = w.saturating_sub(overhead);
                    if col_count == 3 {
                        // 40% / 30% / 30%
                        let c0 = available * 40 / 100;
                        let c1 = available * 30 / 100;
                        let c2 = available - c0 - c1;
                        table.with(Modify::new(Column::from(0)).with(Width::wrap(c0)));
                        table.with(Modify::new(Column::from(1)).with(Width::wrap(c1)));
                        table.with(Modify::new(Column::from(2)).with(Width::wrap(c2)));
                        table.with(Modify::new(Column::from(0)).with(Width::increase(c0)));
                        table.with(Modify::new(Column::from(1)).with(Width::increase(c1)));
                        table.with(Modify::new(Column::from(2)).with(Width::increase(c2)));
                    } else {
                        // Fallback: distribute evenly
                        let each = available / col_count.max(1);
                        for i in 0..col_count {
                            table.with(Modify::new(Column::from(i)).with(Width::wrap(each)));
                            table.with(Modify::new(Column::from(i)).with(Width::increase(each)));
                        }
                    }
                    table
                }
                TableStyle::KeyValue { title } => {
                    let mut builder = Builder::default();
                    for row in rows {
                        builder.push_record(row.iter().map(String::as_str));
                    }
                    let mut table = builder.build();

                    let w = term_width();
                    let available = w.saturating_sub(7); // 3 borders + 4 padding
                    let col0 = available / 3;
                    let col1 = available - col0;

                    // modern() has horizontal lines between ALL rows
                    table.with(Style::modern());

                    // Fixed 1/3 and 2/3 column widths
                    table.with(Modify::new(Column::from(0)).with(Width::increase(col0)));
                    table.with(Modify::new(Column::from(0)).with(Width::wrap(col0)));
                    table.with(Modify::new(Column::from(1)).with(Width::increase(col1)));
                    table.with(Modify::new(Column::from(1)).with(Width::wrap(col1)));

                    // Item name on top border (skip if empty)
                    if !title.is_empty() {
                        table.with(LineText::new(format!(" {title} "), Rows::first()).offset(1));
                    }

                    table
                }
            };

            let rendered = table.to_string();
            let extra_newline = matches!(style, TableStyle::KeyValue { .. });
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
            if extra_newline {
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
