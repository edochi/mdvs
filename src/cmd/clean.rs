use std::path::Path;

pub fn run(path: &Path) -> anyhow::Result<()> {
    let mdvs_dir = path.join(".mdvs");
    if mdvs_dir.exists() {
        std::fs::remove_dir_all(&mdvs_dir)?;
        eprintln!("Removed {}", mdvs_dir.display());
    } else {
        eprintln!("Nothing to clean — {} does not exist", mdvs_dir.display());
    }
    Ok(())
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
