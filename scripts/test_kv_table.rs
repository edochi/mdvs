#!/usr/bin/env -S cargo +nightly -Zscript
---
[dependencies]
tabled = "0.20"
terminal_size = "0.4"
---

use tabled::builder::Builder;
use tabled::settings::{
    object::{Column, Rows},
    style::{HorizontalLine, LineText, Style},
    width::Width,
    Modify,
};

fn term_width() -> usize {
    terminal_size::terminal_size()
        .map(|(terminal_size::Width(w), _)| w as usize)
        .unwrap_or(80)
}

fn main() {
    let w = term_width();
    let available = w.saturating_sub(7);
    let half = available / 2;

    // Field with constraints
    let mut b = Builder::default();
    b.push_record(["type", "Array(String)"]);
    b.push_record(["files", "9 out of 43"]);
    b.push_record(["nullable", "true"]);
    b.push_record(["required", "meetings/all-hands/**\nprojects/alpha/meetings/**\nprojects/beta/meetings/**"]);
    b.push_record(["allowed", "meetings/**\nprojects/alpha/meetings/**\nprojects/beta/meetings/**"]);

    let sep = HorizontalLine::inherit(Style::modern());
    let mut table = b.build();
    table.with(
        Style::rounded().horizontals([
            (1, sep.clone()),
            (2, sep.clone()),
            (3, sep.clone()),
            (4, sep.clone()),
        ])
    );
    table.with(Modify::new(Column::from(0)).with(Width::increase(half)));
    table.with(Modify::new(Column::from(0)).with(Width::wrap(half)));
    table.with(Modify::new(Column::from(1)).with(Width::increase(half)));
    table.with(Modify::new(Column::from(1)).with(Width::wrap(half)));
    table.with(LineText::new(" action_items ", Rows::first()).offset(1));

    println!("{table}");
    println!();

    // Simple field
    let mut b2 = Builder::default();
    b2.push_record(["type", "String"]);
    b2.push_record(["files", "43 out of 43"]);
    b2.push_record(["nullable", "false"]);
    b2.push_record(["required", "(none)"]);
    b2.push_record(["allowed", "**"]);

    let mut table2 = b2.build();
    table2.with(
        Style::rounded().horizontals([
            (1, sep.clone()),
            (2, sep.clone()),
            (3, sep.clone()),
            (4, sep.clone()),
        ])
    );
    table2.with(Modify::new(Column::from(0)).with(Width::increase(half)));
    table2.with(Modify::new(Column::from(0)).with(Width::wrap(half)));
    table2.with(Modify::new(Column::from(1)).with(Width::increase(half)));
    table2.with(Modify::new(Column::from(1)).with(Width::wrap(half)));
    table2.with(LineText::new(" title ", Rows::first()).offset(1));

    println!("{table2}");
}
