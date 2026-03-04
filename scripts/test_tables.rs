#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! tabled = "0.20"
//! terminal_size = "0.4"
//! ```

use tabled::{
    settings::{
        object::Cell,
        span::ColumnSpan,
        themes::BorderCorrection,
        style::Style,
        width::Width,
        Modify,
    },
    builder::Builder,
};
use terminal_size::{terminal_size, Width as TermWidth};

fn term_width() -> usize {
    terminal_size().map(|(TermWidth(w), _)| w as usize).unwrap_or(80)
}

fn style_compact(table: &mut tabled::Table) {
    let w = term_width();
    table.with(Style::rounded().remove_horizontals());
    table.with(Width::increase(w));
    table.with(Width::wrap(w));
}

fn style_record(table: &mut tabled::Table, cols: isize) {
    let w = term_width();
    table.with(Style::rounded());
    table.with(Modify::new(Cell::new(1, 0)).with(ColumnSpan::new(cols)));
    table.with(BorderCorrection {});
    table.with(Width::increase(w));
    table.with(Width::wrap(w));
}

// ── Compact table: check violations ──────────────────────────────────────────

fn compact_check() {
    println!("Checked 498 files — 3 violation(s)\n");

    let mut builder = Builder::default();
    builder.push_record(["\"result\"", "MissingRequired", "2 files"]);
    builder.push_record(["\"result\"", "WrongType", "1 file"]);
    builder.push_record(["\"status\"", "Disallowed", "3 files"]);

    let mut table = builder.build();
    style_compact(&mut table);
    println!("{table}");
}

// ── Compact table: search results ────────────────────────────────────────────

fn compact_search() {
    println!("\nSearched \"rust\" — 10 hits\n");

    let mut builder = Builder::default();
    builder.push_record(["1", "\"projects/Core Language - Rust.md\"", "0.656"]);
    builder.push_record(["2", "\"technologies/rust/notes/Atomic Types.md\"", "0.655"]);
    builder.push_record(["3", "\"articles/rust-guide.md\"", "0.612"]);
    builder.push_record(["4", "\"books/Programming Rust/ch1.md\"", "0.590"]);
    builder.push_record(["5", "\"projects/rustlings/notes.md\"", "0.543"]);

    let mut table = builder.build();
    style_compact(&mut table);
    println!("{table}");
}

// ── Compact table: init fields ───────────────────────────────────────────────

fn compact_init() {
    println!("\nInitialized 498 files — 10 fields\n");

    let mut builder = Builder::default();
    builder.push_record(["\"author\"", "String", "45/498"]);
    builder.push_record(["\"tags\"", "Array", "120/498"]);
    builder.push_record(["\"draft\"", "Boolean", "30/498"]);
    builder.push_record(["\"status\"", "String", "80/498"]);

    let mut table = builder.build();
    style_compact(&mut table);
    println!("{table}");
}

// ── Compact table: info ──────────────────────────────────────────────────────

fn compact_info() {
    println!("\n498 files, 10 fields, 2314 chunks\n");

    let mut builder = Builder::default();
    builder.push_record(["model:", "minishlab/potion-base-8M"]);
    builder.push_record(["config:", "match"]);
    builder.push_record(["files:", "498/498"]);

    let mut table = builder.build();
    style_compact(&mut table);
    println!("{table}");

    println!();

    let mut builder2 = Builder::default();
    builder2.push_record(["\"author\"", "String", "required: \"articles/**\"", "allowed: \"articles/**\""]);
    builder2.push_record(["\"tags\"", "Array", "required: \"articles/**\", ...", "allowed: \"**\""]);
    builder2.push_record(["\"draft\"", "Boolean", "", "allowed: \"**\""]);

    let mut table2 = builder2.build();
    style_compact(&mut table2);
    println!("{table2}");
}

// ── Verbose record table: check violations ───────────────────────────────────

fn verbose_check_record() {
    println!("\n── Verbose: check ──\n");
    println!("Checked 498 files — 3 violation(s)\n");

    let mut b1 = Builder::default();
    b1.push_record(["\"result\"", "MissingRequired", "2 files"]);
    b1.push_record(["  - \"books/High Performance Browser Networking/ch1.md\"\n  - \"books/Introduction to Neuromorphic Computing/notes.md\"", "", ""]);
    let mut t1 = b1.build();
    style_record(&mut t1, 3);
    println!("{t1}");

    let mut b2 = Builder::default();
    b2.push_record(["\"result\"", "WrongType", "1 file"]);
    b2.push_record(["  - \"articles/foo.md\" (got Integer)", "", ""]);
    let mut t2 = b2.build();
    style_record(&mut t2, 3);
    println!("{t2}");

    let mut b3 = Builder::default();
    b3.push_record(["\"status\"", "Disallowed", "3 files"]);
    b3.push_record(["  - \"projects/old/draft.md\"\n  - \"projects/old/notes.md\"\n  - \"projects/old/todo.md\"", "", ""]);
    let mut t3 = b3.build();
    style_record(&mut t3, 3);
    println!("{t3}");

    println!("498 files | glob: \"**\" | 200ms");
}

// ── Verbose record table: search results ─────────────────────────────────────

fn verbose_search_record() {
    println!("\n── Verbose: search ──\n");
    println!("Searched \"rust\" — 10 hits\n");

    let chunk1 = "  lines 1-4:\n    Rust is a systems programming language focused on safety\n    and performance. It achieves memory safety without garbage\n    collection through its ownership system.";
    let chunk2 = "  lines 1-6:\n    Atomic types provide lock-free concurrent access to shared\n    data. The std::sync::atomic module exposes AtomicBool,\n    AtomicUsize, and other atomic primitives.";
    let chunk3 = "  lines 8-14:\n    The borrow checker is Rust's key innovation. It enforces\n    that references follow strict aliasing rules at compile\n    time, preventing data races entirely.";

    for (i, (path, score, chunk)) in [
        ("\"projects/Core Language - Rust.md\"", "0.656", chunk1),
        ("\"technologies/rust/notes/Atomic Types.md\"", "0.655", chunk2),
        ("\"articles/rust-guide.md\"", "0.612", chunk3),
    ].iter().enumerate() {
        let mut b = Builder::default();
        let idx = format!("{}", i + 1);
        b.push_record([idx.as_str(), path, score]);
        b.push_record([chunk, "", ""]);
        let mut t = b.build();
        style_record(&mut t, 3);
        println!("{t}");
    }

    println!("10 hits | model: \"minishlab/potion-base-8M\" | limit: 10 | 580ms");
}

// ── Verbose record table: build incremental ──────────────────────────────────

fn verbose_build_record() {
    println!("\n── Verbose: build (incremental) ──\n");
    println!("Built index — 498 files, 2314 chunks\n");

    let mut b1 = Builder::default();
    b1.push_record(["embedded", "120 files", "800 chunks"]);
    b1.push_record(["  - \"articles/new-post.md\" (6 chunks)\n  - \"articles/updated-post.md\" (4 chunks)\n  - ...", "", ""]);
    let mut t1 = b1.build();
    style_record(&mut t1, 3);
    println!("{t1}");

    let mut b2 = Builder::default();
    b2.push_record(["unchanged", "370 files", "1514 chunks"]);
    let mut t2 = b2.build();
    style_compact(&mut t2);
    println!("{t2}");

    let mut b3 = Builder::default();
    b3.push_record(["removed", "8 files", "42 chunks"]);
    b3.push_record(["  - \"archive/old-post.md\" (5 chunks)\n  - \"archive/deprecated.md\" (3 chunks)\n  - ...", "", ""]);
    let mut t3 = b3.build();
    style_record(&mut t3, 3);
    println!("{t3}");

    println!("498 files | model: \"minishlab/potion-base-8M\" | glob: \"**\" | 540ms");
}

// ── Verbose record table: info ───────────────────────────────────────────────

fn verbose_info_record() {
    println!("\n── Verbose: info ──\n");
    println!("498 files, 10 fields, 2314 chunks\n");

    let mut bm = Builder::default();
    bm.push_record(["model:", "minishlab/potion-base-8M"]);
    bm.push_record(["revision:", "abc123"]);
    bm.push_record(["chunk size:", "1024"]);
    bm.push_record(["built:", "2026-03-04T00:24:55+00:00"]);
    bm.push_record(["config:", "match"]);
    bm.push_record(["files:", "498/498"]);
    let mut tm = bm.build();
    style_compact(&mut tm);
    println!("{tm}");
    println!();

    let fields = [
        ("\"author\"", "String", "45/498", "  required:\n    - \"articles/**\"\n  allowed:\n    - \"articles/**\""),
        ("\"tags\"", "Array", "120/498", "  required:\n    - \"articles/**\"\n    - \"books/**\"\n    - \"questions/**\"\n  allowed: \"**\""),
        ("\"draft\"", "Boolean", "30/498", "  allowed: \"**\""),
    ];

    for (name, typ, count, detail) in &fields {
        let mut b = Builder::default();
        b.push_record([*name, *typ, *count]);
        b.push_record([*detail, "", ""]);
        let mut t = b.build();
        style_record(&mut t, 3);
        println!("{t}");
    }

    println!("498 files | glob: \"**\" | 50ms");
}

fn main() {
    println!("Terminal width: {}\n", term_width());

    println!("═══════════════════════════════════════");
    println!("  COMPACT FORMAT");
    println!("═══════════════════════════════════════\n");

    compact_check();
    compact_search();
    compact_init();
    compact_info();

    println!("\n═══════════════════════════════════════");
    println!("  VERBOSE FORMAT");
    println!("═══════════════════════════════════════");

    verbose_check_record();
    verbose_search_record();
    verbose_build_record();
    verbose_info_record();
}
