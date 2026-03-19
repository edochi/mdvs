use crate::index::backend::Backend;
use crate::outcome::commands::CleanOutcome;
use crate::outcome::{DeleteIndexOutcome, Outcome};
use crate::step::{ErrorKind, Step, StepError, StepOutcome};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::instrument;

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
pub fn run(path: &Path) -> Step<Outcome> {
    let start = Instant::now();
    let mut substeps = Vec::new();

    // Delete index step — inlined from pipeline/delete_index.rs
    let delete_start = Instant::now();
    let mdvs_dir = path.join(".mdvs");

    if mdvs_dir.is_symlink() {
        substeps.push(Step::failed(
            ErrorKind::User,
            format!(
                "'{}' is a symlink — refusing to delete for safety",
                mdvs_dir.display()
            ),
            delete_start.elapsed().as_millis() as u64,
        ));
        let msg = format!(
            "'{}' is a symlink — refusing to delete for safety",
            mdvs_dir.display()
        );
        return Step {
            substeps,
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: msg,
                }),
                elapsed_ms: start.elapsed().as_millis() as u64,
            },
        };
    }

    let (removed, path_str, files_removed, size_bytes) = if mdvs_dir.exists() {
        let (files_removed, size_bytes) = match walk_dir_stats(&mdvs_dir) {
            Ok(stats) => stats,
            Err(e) => {
                substeps.push(Step::failed(
                    ErrorKind::Application,
                    e.to_string(),
                    delete_start.elapsed().as_millis() as u64,
                ));
                return Step {
                    substeps,
                    outcome: StepOutcome::Complete {
                        result: Err(StepError {
                            kind: ErrorKind::Application,
                            message: e.to_string(),
                        }),
                        elapsed_ms: start.elapsed().as_millis() as u64,
                    },
                };
            }
        };

        let backend = Backend::parquet(path);
        if let Err(e) = backend.clean() {
            substeps.push(Step::failed(
                ErrorKind::Application,
                e.to_string(),
                delete_start.elapsed().as_millis() as u64,
            ));
            return Step {
                substeps,
                outcome: StepOutcome::Complete {
                    result: Err(StepError {
                        kind: ErrorKind::Application,
                        message: e.to_string(),
                    }),
                    elapsed_ms: start.elapsed().as_millis() as u64,
                },
            };
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

    substeps.push(Step::leaf(
        Outcome::DeleteIndex(DeleteIndexOutcome {
            removed,
            path: path_str.clone(),
            files_removed,
            size_bytes,
        }),
        delete_start.elapsed().as_millis() as u64,
    ));

    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Clean(CleanOutcome {
                removed,
                path: PathBuf::from(&path_str),
                files_removed,
                size_bytes,
            })),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::Outcome;
    use crate::step::StepOutcome;
    use std::fs;

    fn unwrap_clean(step: &Step<Outcome>) -> &CleanOutcome {
        match &step.outcome {
            StepOutcome::Complete {
                result: Ok(Outcome::Clean(o)),
                ..
            } => o,
            other => panic!("expected Ok(Clean), got: {other:?}"),
        }
    }

    #[test]
    fn clean_removes_mdvs_dir() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("mdvs.toml"), "[scan]\nglob = \"**\"\n").unwrap();
        let mdvs_dir = tmp.path().join(".mdvs");
        fs::create_dir_all(&mdvs_dir).unwrap();
        fs::write(mdvs_dir.join("files.parquet"), "dummy").unwrap();

        let step = run(tmp.path());
        assert!(!crate::step::has_failed(&step));

        let result = unwrap_clean(&step);
        assert!(result.removed);
        assert!(!mdvs_dir.exists());
        assert_eq!(result.files_removed, 1);
        assert_eq!(result.size_bytes, 5);
        assert!(tmp.path().join("mdvs.toml").exists());
    }

    #[test]
    fn clean_nothing_to_clean() {
        let tmp = tempfile::tempdir().unwrap();

        let step = run(tmp.path());
        assert!(!crate::step::has_failed(&step));

        let result = unwrap_clean(&step);
        assert!(!result.removed);
        assert_eq!(result.files_removed, 0);
        assert_eq!(result.size_bytes, 0);
    }
}
