mod check;
mod diff;
mod init;
mod update;

pub use check::cmd_check;
pub use diff::cmd_diff;
pub use init::cmd_init;
pub use update::cmd_update;

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Derive the lock file path from a config path: `foo.toml` → `foo.lock`.
pub(crate) fn lock_path_for(config_path: &Path) -> PathBuf {
    config_path.with_extension("lock")
}

/// Resolve schema path by precedence:
/// 1. Explicit --schema path
/// 2. {dir}/mfv.toml
/// 3. {dir}/mdvs.toml
/// 4. Error
pub(crate) fn resolve_schema_path(dir: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    let mfv_toml = dir.join("mfv.toml");
    if mfv_toml.is_file() {
        return Ok(mfv_toml);
    }

    let mdvs_toml = dir.join("mdvs.toml");
    if mdvs_toml.is_file() {
        return Ok(mdvs_toml);
    }

    bail!(
        "no config found; provide --schema or create mfv.toml / mdvs.toml in {}",
        dir.display()
    )
}
