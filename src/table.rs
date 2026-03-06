//! Table rendering helpers for command output.
//!
//! Provides two table styles — compact (no internal lines) and record (detail row
//! spanning all columns) — both using rounded borders and auto-sized to terminal width.

use tabled::settings::{
    object::Cell, peaker::PriorityMax, span::ColumnSpan, style::Style, themes::BorderCorrection,
    width::Width, Modify,
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
