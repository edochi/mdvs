//! Rendering primitives and the `Render` trait.
//!
//! Data types produce `Vec<Block>` via the `Render` trait. Shared formatters
//! (`format_text`, `format_markdown`) consume blocks and produce formatted
//! output strings. This separation means adding a new output format requires
//! writing one formatter function, not touching any command or outcome type.

/// A rendering primitive — the intermediate representation between data and
/// formatted output.
#[derive(Debug, Clone)]
pub enum Block {
    /// A single line of text.
    Line(String),
    /// A table with optional headers and styled rows.
    Table {
        /// Column headers (displayed as the first row in most formats).
        headers: Option<Vec<String>>,
        /// Table rows, each a vector of cell strings.
        rows: Vec<Vec<String>>,
        /// How the table should be styled by the formatter.
        style: TableStyle,
    },
    /// A labeled group of child blocks (for nested command output, sections).
    Section {
        /// Section header label.
        label: String,
        /// Child blocks within this section.
        children: Vec<Block>,
    },
}

/// Table styling hints consumed by formatters.
#[derive(Debug, Clone)]
pub enum TableStyle {
    /// No internal horizontal separators. For compact summary tables.
    Compact,
    /// Detail rows span all columns (via Panel in text formatter).
    /// For per-item record tables with expandable detail.
    Record {
        /// Zero-based row indices that should span all columns as detail rows.
        detail_rows: Vec<usize>,
    },
    /// Two-column key-value table with item name on top border.
    /// Horizontal separators between all rows. Fixed 50/50 column widths.
    KeyValue {
        /// Item name displayed on the top border.
        title: String,
    },
}

/// Trait for types that render themselves as a sequence of blocks.
///
/// No parameters — the struct IS the verbose/compact decision. `Outcome`
/// structs produce full detail blocks; `CompactOutcome` structs produce
/// summary blocks or empty vecs (for silent leaf steps).
pub trait Render {
    /// Produce rendering blocks for this type.
    fn render(&self) -> Vec<Block>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_line() {
        let block = Block::Line("hello".into());
        match block {
            Block::Line(s) => assert_eq!(s, "hello"),
            _ => panic!("expected Line"),
        }
    }

    #[test]
    fn block_table() {
        let block = Block::Table {
            headers: Some(vec!["name".into(), "type".into()]),
            rows: vec![vec!["title".into(), "String".into()]],
            style: TableStyle::Compact,
        };
        match block {
            Block::Table { headers, rows, .. } => {
                assert_eq!(headers.unwrap().len(), 2);
                assert_eq!(rows.len(), 1);
            }
            _ => panic!("expected Table"),
        }
    }

    #[test]
    fn block_section() {
        let block = Block::Section {
            label: "Auto-build".into(),
            children: vec![Block::Line("Scan: 5 files".into())],
        };
        match block {
            Block::Section { label, children } => {
                assert_eq!(label, "Auto-build");
                assert_eq!(children.len(), 1);
            }
            _ => panic!("expected Section"),
        }
    }

    #[test]
    fn block_is_clone() {
        let block = Block::Line("test".into());
        let cloned = block.clone();
        match cloned {
            Block::Line(s) => assert_eq!(s, "test"),
            _ => panic!("expected Line"),
        }
    }

    struct DummyOutcome {
        label: String,
    }

    impl Render for DummyOutcome {
        fn render(&self) -> Vec<Block> {
            vec![Block::Line(self.label.clone())]
        }
    }

    #[test]
    fn render_trait_on_dummy() {
        let outcome = DummyOutcome {
            label: "Scan: 5 files".into(),
        };
        let blocks = outcome.render();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            Block::Line(s) => assert_eq!(s, "Scan: 5 files"),
            _ => panic!("expected Line"),
        }
    }
}
