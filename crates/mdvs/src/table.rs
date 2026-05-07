//! Table rendering helpers for command output.
//!
//! Provides two table styles — compact (no internal lines) and record (detail row
//! spanning all columns) — both using rounded borders and auto-sized to terminal width.

use tabled::settings::{
    Modify, object::Cell, peaker::PriorityMax, span::ColumnSpan, style::Style,
    themes::BorderCorrection, width::Width,
};

pub use tabled::builder::Builder;

/// Detect terminal width, falling back to 80 columns.
pub fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(terminal_size::Width(w), _)| w as usize)
        .unwrap_or(80)
}

/// Apply compact table style: rounded borders, no internal horizontal lines,
/// stretched to terminal width.
pub fn style_compact(table: &mut tabled::Table) {
    let w = term_width();
    table.with(Style::rounded().remove_horizontals());
    table.with(Width::increase(w));
    table.with(Width::wrap(w).priority(PriorityMax::left()));
}

/// Apply record table style: rounded borders, second row spans all columns
/// (for multi-line detail text), stretched to terminal width.
pub fn style_record(table: &mut tabled::Table, cols: isize) {
    let w = term_width();
    table.with(Style::rounded());
    table.with(Modify::new(Cell::new(1, 0)).with(ColumnSpan::new(cols)));
    table.with(BorderCorrection {});
    table.with(Width::increase(w));
    table.with(Width::wrap(w).priority(PriorityMax::left()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn style_compact_smoke() {
        let mut builder = Builder::default();
        builder.push_record(["name", "type", "count"]);
        builder.push_record(["title", "String", "5/5"]);
        builder.push_record(["draft", "Boolean", "3/5"]);
        let mut table = builder.build();
        style_compact(&mut table);
        let rendered = table.to_string();
        assert!(rendered.contains("title"));
        assert!(rendered.contains("Boolean"));
    }

    #[test]
    fn style_record_smoke() {
        let mut builder = Builder::default();
        builder.push_record(["\"title\"", "String", "5/5"]);
        builder.push_record(["  required:\n    - \"**\"", "", ""]);
        let mut table = builder.build();
        style_record(&mut table, 3);
        let rendered = table.to_string();
        assert!(rendered.contains("title"));
        assert!(rendered.contains("required"));
    }

    #[test]
    fn style_compact_empty_table() {
        let builder = Builder::default();
        let mut table = builder.build();
        style_compact(&mut table);
        // Should not panic on empty table
        let _ = table.to_string();
    }

    #[test]
    fn term_width_returns_positive() {
        let w = term_width();
        assert!(w > 0);
    }
}
