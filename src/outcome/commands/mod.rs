//! Outcome types for command-level results.
//!
//! One file per command. Each defines a full + compact outcome pair
//! with `Render` and `From` impls.

pub mod build;
pub mod check;
pub mod clean;
pub mod info;
pub mod init;
pub mod search;
pub mod update;

pub use build::{BuildOutcome, BuildOutcomeCompact};
pub use check::{CheckOutcome, CheckOutcomeCompact};
pub use clean::{CleanOutcome, CleanOutcomeCompact};
pub use info::{InfoOutcome, InfoOutcomeCompact};
pub use init::{InitOutcome, InitOutcomeCompact};
pub use search::{SearchOutcome, SearchOutcomeCompact};
pub use update::{UpdateOutcome, UpdateOutcomeCompact};
