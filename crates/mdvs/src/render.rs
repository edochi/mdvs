//! Shared formatters that consume `Vec<Block>` and produce formatted output.
//!
//! Two formatters today: `format_pretty` (terminal, box-drawing tables via
//! tabled) and `format_markdown` (GFM pipe tables, `##` section headers).
//! Adding a new output format means writing one function here — no command
//! code changes needed.

use tabled::settings::{
    Modify, Panel,
    object::{Column, Rows},
    style::{LineText, Style},
    themes::BorderCorrection,
    width::Width,
};

use crate::block::{Block, TableStyle};
use crate::table::{Builder, style_compact, term_width};

/// Format blocks as terminal-friendly pretty output with box-drawing tables.
pub fn format_pretty(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        format_pretty_block(block, &mut out, 0);
    }
    out
}

fn format_pretty_block(block: &Block, out: &mut String, indent: usize) {
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
                        // `detail_rows` is caller-supplied; in normal use it
                        // indexes into `rows` and every row has at least one
                        // column. Skip silently if either invariant fails so
                        // a future refactor of the caller can't crash table
                        // rendering — Panel skipping just means no detail
                        // pane for that row.
                        let Some(detail_text) = rows
                            .get(row_idx)
                            .and_then(|r| r.first())
                            .filter(|s| !s.is_empty())
                        else {
                            continue;
                        };
                        // Count non-detail rows before this detail row
                        let data_rows_before =
                            (0..row_idx).filter(|i| !detail_rows.contains(i)).count();
                        // Insert position: after the last data row + header + previously inserted panels
                        let pos = data_rows_before + header_offset + panels_inserted;
                        table.with(Panel::horizontal(pos, detail_text));
                        panels_inserted += 1;
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
                format_pretty_block(child, out, indent + 2);
            }
        }
    }
}

/// Format blocks as markdown (pipe tables, section headers).
///
/// Renders `Block::Line` as a paragraph, `Block::Section` as a `##` heading +
/// children, and `Block::Table` differently per style:
///
/// - `KeyValue { title }` — `### title` (when non-empty) plus a 2-column table
///   with `Field | Value` headers. Used by every Outcome today.
/// - `Compact` / `Record` — pipe table with the supplied headers if any.
///   Detail rows from `Record` are rendered as plain rows; mdvs's current
///   Outcomes don't use these styles.
///
/// Cells escape `|`, `\`, and convert embedded newlines to `<br>` so multiline
/// values (e.g. lists of glob patterns) render correctly in GFM tables.
///
/// The output does not end in a trailing newline (call sites use `print!`,
/// not `println!`).
pub fn format_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    for block in blocks {
        format_markdown_block(block, &mut out);
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

fn format_markdown_block(block: &Block, out: &mut String) {
    match block {
        Block::Line(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Block::Table {
            headers,
            rows,
            style,
        } => match style {
            TableStyle::KeyValue { title } => {
                if !title.is_empty() {
                    out.push_str("### ");
                    out.push_str(title);
                    out.push_str("\n\n");
                }
                out.push_str("| Field | Value |\n| --- | --- |\n");
                for row in rows {
                    out.push_str("| ");
                    out.push_str(
                        &row.iter()
                            .map(|c| escape_cell(c))
                            .collect::<Vec<_>>()
                            .join(" | "),
                    );
                    out.push_str(" |\n");
                }
                out.push('\n');
            }
            TableStyle::Compact | TableStyle::Record { .. } => {
                if let Some(hdrs) = headers {
                    out.push_str("| ");
                    out.push_str(
                        &hdrs
                            .iter()
                            .map(|h| escape_cell(h))
                            .collect::<Vec<_>>()
                            .join(" | "),
                    );
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
                    out.push_str(
                        &row.iter()
                            .map(|c| escape_cell(c))
                            .collect::<Vec<_>>()
                            .join(" | "),
                    );
                    out.push_str(" |\n");
                }
                out.push('\n');
            }
        },
        Block::Section { label, children } => {
            out.push_str("## ");
            out.push_str(label);
            out.push_str("\n\n");
            for child in children {
                format_markdown_block(child, out);
            }
        }
    }
}

/// Escape a cell value for inclusion in a GFM pipe table.
///
/// `\` becomes `\\`, `|` becomes `\|`, and newlines become `<br>`. Other
/// markdown specials (`*`, `_`, etc.) are left as-is — callers that produce
/// `Block::Table` cells supply plain strings, not pre-formatted markdown.
fn escape_cell(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '|' => out.push_str("\\|"),
            '\n' => out.push_str("<br>"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::Block;

    #[test]
    fn pretty_empty_blocks() {
        assert_eq!(format_pretty(&[]), "");
    }

    #[test]
    fn pretty_line() {
        let blocks = vec![Block::Line("hello world".into())];
        assert_eq!(format_pretty(&blocks), "hello world\n");
    }

    #[test]
    fn pretty_multiple_lines() {
        let blocks = vec![Block::Line("line 1".into()), Block::Line("line 2".into())];
        assert_eq!(format_pretty(&blocks), "line 1\nline 2\n");
    }

    #[test]
    fn pretty_compact_table() {
        let blocks = vec![Block::Table {
            headers: Some(vec!["name".into(), "type".into()]),
            rows: vec![vec!["title".into(), "String".into()]],
            style: TableStyle::Compact,
        }];
        let output = format_pretty(&blocks);
        assert!(output.contains("title"));
        assert!(output.contains("String"));
        // Rounded border chars
        assert!(output.contains('╭') || output.contains('│'));
    }

    #[test]
    fn pretty_record_table() {
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
        let output = format_pretty(&blocks);
        assert!(output.contains("title"));
        assert!(output.contains("required"));
    }

    #[test]
    fn pretty_section() {
        let blocks = vec![Block::Section {
            label: "Auto-build".into(),
            children: vec![Block::Line("Scan: 5 files".into())],
        }];
        let output = format_pretty(&blocks);
        assert!(output.contains("Auto-build:"));
        assert!(output.contains("  Scan: 5 files"));
    }

    #[test]
    fn markdown_line() {
        let blocks = vec![Block::Line("hello".into())];
        assert_eq!(format_markdown(&blocks), "hello");
    }

    #[test]
    fn markdown_no_trailing_newline() {
        let blocks = vec![Block::Line("hello".into()), Block::Line("world".into())];
        let output = format_markdown(&blocks);
        assert!(!output.ends_with('\n'));
        assert!(output.ends_with("world"));
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

    #[test]
    fn markdown_keyvalue_with_title() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![
                vec!["type".into(), "Array(String)".into()],
                vec!["nullable".into(), "false".into()],
            ],
            style: TableStyle::KeyValue {
                title: "action_items".into(),
            },
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("### action_items"));
        assert!(output.contains("| Field | Value |"));
        assert!(output.contains("| --- | --- |"));
        assert!(output.contains("| type | Array(String) |"));
        assert!(output.contains("| nullable | false |"));
    }

    #[test]
    fn markdown_keyvalue_without_title() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![vec!["scan glob".into(), "**".into()]],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        }];
        let output = format_markdown(&blocks);
        assert!(!output.contains("###"));
        assert!(output.contains("| Field | Value |"));
        assert!(output.contains("| scan glob | ** |"));
    }

    #[test]
    fn markdown_cell_escapes_pipe() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![vec!["pattern".into(), "foo|bar".into()]],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("foo\\|bar"));
    }

    #[test]
    fn markdown_cell_converts_newlines_to_br() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![vec!["required".into(), "globs/**\nother/**".into()]],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("globs/**<br>other/**"));
        // No literal newline mid-row — the row stays on one line.
        let row_line = output
            .lines()
            .find(|l| l.contains("required"))
            .expect("row");
        assert!(!row_line.contains('\n'));
    }

    #[test]
    fn markdown_cell_escapes_backslash() {
        let blocks = vec![Block::Table {
            headers: None,
            rows: vec![vec!["path".into(), "a\\b".into()]],
            style: TableStyle::KeyValue {
                title: String::new(),
            },
        }];
        let output = format_markdown(&blocks);
        assert!(output.contains("a\\\\b"));
    }
}
