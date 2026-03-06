use crate::index::backend::Backend;
use crate::output::{format_file_count, format_size, CommandOutput};
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
    /// Number of files that were in `.mdvs/` before deletion.
    pub files_removed: usize,
    /// Total size of `.mdvs/` in bytes before deletion.
    pub size_bytes: u64,
    /// Wall-clock time for the operation in milliseconds.
    pub elapsed_ms: u64,
}

impl CommandOutput for CleanResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();
        if self.removed {
            out.push_str(&format!("Cleaned \"{}\"\n", self.path.display()));
            if verbose {
                out.push_str(&format!(
                    "\n{} | {} | {}ms\n",
                    format_file_count(self.files_removed),
                    format_size(self.size_bytes),
                    self.elapsed_ms
                ));
            }
        } else {
            out.push_str(&format!(
                "Nothing to clean — \"{}\" does not exist\n",
                self.path.display()
            ));
        }
        out
    }
}

/// Count files and sum their sizes in a directory (recursively).
fn walk_dir_stats(dir: &Path) -> anyhow::Result<(usize, u64)> {
    let mut count = 0usize;
    let mut size = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_dir() {
            let (c, s) = walk_dir_stats(&entry.path())?;
            count += c;
            size += s;
        } else {
            count += 1;
            size += meta.len();
        }
    }
    Ok((count, size))
}

/// Delete the `.mdvs/` index directory if it exists.
#[instrument(name = "clean", skip_all)]
pub fn run(path: &Path) -> anyhow::Result<CleanResult> {
    let start = std::time::Instant::now();
    let mdvs_dir = path.join(".mdvs");
    if mdvs_dir.exists() {
        let (files_removed, size_bytes) = walk_dir_stats(&mdvs_dir)?;
        let backend = Backend::parquet(path, "_");
        backend.clean()?;
        Ok(CleanResult {
            removed: true,
            path: mdvs_dir,
            files_removed,
            size_bytes,
            elapsed_ms: start.elapsed().as_millis() as u64,
        })
    } else {
        Ok(CleanResult {
            removed: false,
            path: mdvs_dir,
            files_removed: 0,
            size_bytes: 0,
            elapsed_ms: start.elapsed().as_millis() as u64,
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

        let result = run(tmp.path()).unwrap();
        assert!(result.removed);
        assert!(!mdvs_dir.exists());
        assert_eq!(result.files_removed, 1);
        assert_eq!(result.size_bytes, 5); // "dummy" is 5 bytes
                                          // mdvs.toml should be untouched
        assert!(tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn clean_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();

        let result = run(tmp.path()).unwrap();
        assert!(!result.removed);
        assert_eq!(result.files_removed, 0);
        assert_eq!(result.size_bytes, 0);
    }
}
