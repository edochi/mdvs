use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use tracing::instrument;

use crate::{
    index::backend::Backend,
    outcome::{DeleteIndexOutcome, Outcome, commands::CleanOutcome},
    step::{CommandResult, ErrorKind, StepEntry, elapsed_ms},
};

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
pub async fn run(path: &Path) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // Delete index step — inlined from pipeline/delete_index.rs
    let delete_start = Instant::now();
    let mdvs_dir = path.join(".mdvs");

    if mdvs_dir.is_symlink() {
        let msg = format!(
            "'{}' is a symlink — refusing to delete for safety",
            mdvs_dir.display()
        );
        steps.push(StepEntry::err(
            ErrorKind::User,
            msg.clone(),
            elapsed_ms(delete_start),
        ));
        return CommandResult::failed(steps, ErrorKind::User, msg, start);
    }

    let (removed, path_str, files_removed, size_bytes) = if mdvs_dir.exists() {
        let (files_removed, size_bytes) = match walk_dir_stats(&mdvs_dir) {
            Ok(stats) => stats,
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    e.to_string(),
                    elapsed_ms(delete_start),
                ));
                return CommandResult::failed_from_steps(steps, start);
            }
        };

        let backend = Backend::lance(path);
        if let Err(e) = backend.clean().await {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(delete_start),
            ));
            return CommandResult::failed_from_steps(steps, start);
        }

        (
            true,
            mdvs_dir.display().to_string(),
            files_removed,
            size_bytes,
        )
    } else {
        (false, mdvs_dir.display().to_string(), 0, 0)
    };

    steps.push(StepEntry::ok(
        Outcome::DeleteIndex(DeleteIndexOutcome {
            removed,
            path: path_str.clone(),
            files_removed,
            size_bytes,
        }),
        elapsed_ms(delete_start),
    ));

    CommandResult {
        steps,
        result: Ok(Outcome::Clean(CleanOutcome {
            removed,
            path: PathBuf::from(&path_str),
            files_removed,
            size_bytes,
        })),
        elapsed_ms: elapsed_ms(start),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{outcome::Outcome, step::CommandResult};

    fn unwrap_clean(result: &CommandResult) -> &CleanOutcome {
        match &result.result {
            Ok(Outcome::Clean(o)) => o,
            other => panic!("expected Ok(Clean), got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn clean_removes_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("mdvs.toml"), "[scan]\nglob = \"**\"\n").unwrap();
        let mdvs_dir = tmp.path().join(".mdvs");
        fs::create_dir_all(&mdvs_dir).unwrap();
        fs::write(mdvs_dir.join("files.parquet"), "dummy").unwrap();

        let result = run(tmp.path()).await;
        assert!(!crate::step::has_failed(&result));

        let outcome = unwrap_clean(&result);
        assert!(outcome.removed);
        assert!(!mdvs_dir.exists());
        assert_eq!(outcome.files_removed, 1);
        assert_eq!(outcome.size_bytes, 5);
        assert!(tmp.path().join("mdvs.toml").exists());
    }

    #[tokio::test]
    async fn clean_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();

        let result = run(tmp.path()).await;
        assert!(!crate::step::has_failed(&result));

        let outcome = unwrap_clean(&result);
        assert!(!outcome.removed);
        assert_eq!(outcome.files_removed, 0);
        assert_eq!(outcome.size_bytes, 0);
    }
}
