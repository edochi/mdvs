use crate::index::backend::Backend;
use crate::output::CommandOutput;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tracing::instrument;

/// Result of the `clean` command.
#[derive(Debug, Serialize)]
pub struct CleanResult {
    /// Whether the `.mdvs/` directory was actually removed.
    pub removed: bool,
    /// Path to the `.mdvs/` directory.
    pub path: PathBuf,
}

impl CommandOutput for CleanResult {
    fn format_human(&self) -> String {
        if self.removed {
            format!("Removed {}\n", self.path.display())
        } else {
            format!("Nothing to clean — {} does not exist\n", self.path.display())
        }
    }
}

/// Delete the `.mdvs/` index directory if it exists.
#[instrument(name = "clean", skip_all)]
pub fn run(path: &Path) -> anyhow::Result<CleanResult> {
    let mdvs_dir = path.join(".mdvs");
    if mdvs_dir.exists() {
        let backend = Backend::parquet(path, "_");
        backend.clean()?;
        Ok(CleanResult {
            removed: true,
            path: mdvs_dir,
        })
    } else {
        Ok(CleanResult {
            removed: false,
            path: mdvs_dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn clean_removes_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();

        // Create mdvs.toml and .mdvs/ with a dummy file
        fs::write(tmp.path().join("mdvs.toml"), "[scan]\nglob = \"**\"\n").unwrap();
        let mdvs_dir = tmp.path().join(".mdvs");
        fs::create_dir_all(&mdvs_dir).unwrap();
        fs::write(mdvs_dir.join("files.parquet"), "dummy").unwrap();

        let result = run(tmp.path());
        assert!(result.is_ok());
        assert!(!mdvs_dir.exists());
        // mdvs.toml should be untouched
        assert!(tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn clean_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();

        let result = run(tmp.path());
        assert!(result.is_ok());
        // No error, just a message
    }
}
