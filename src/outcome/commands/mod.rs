//! Outcome types for command-level results.
//!
//! One file per command, each defining the outcome struct and its `Render` impl.

pub mod build;
pub mod check;
pub mod clean;
pub mod info;
pub mod init;
pub mod search;
pub mod update;

pub use build::BuildOutcome;
pub use check::CheckOutcome;
pub use clean::CleanOutcome;
pub use info::InfoOutcome;
pub use init::InitOutcome;
pub use search::SearchOutcome;
pub use update::UpdateOutcome;
