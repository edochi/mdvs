use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::outcome::commands::UpdateOutcome;
use crate::outcome::{InferOutcome, Outcome, ReadConfigOutcome, ScanOutcome, WriteConfigOutcome};
use crate::output::{ChangedField, FieldChange, RemovedField};
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::shared::FieldTypeSerde;
use crate::step::{CommandResult, ErrorKind, StepEntry};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

/// Re-scan files, infer field changes, and update `mdvs.toml`.
/// Pure inference — no build step.
#[instrument(name = "update", skip_all)]
pub async fn run(
    path: &Path,
    reinfer: &[String],
    reinfer_all: bool,
    dry_run: bool,
    _verbose: bool,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // Pre-check: flag conflict
    if !reinfer.is_empty() && reinfer_all {
        return CommandResult::failed(
            steps,
            ErrorKind::User,
            "cannot use --reinfer and --reinfer-all together".into(),
            start,
        );
    }

    // 1. Read config — MdvsToml::read() + validate() directly
    let config_start = Instant::now();
    let config_path_buf = path.join("mdvs.toml");
    let mut config = match MdvsToml::read(&config_path_buf) {
        Ok(cfg) => match cfg.validate() {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::ReadConfig(ReadConfigOutcome {
                        config_path: config_path_buf.display().to_string(),
                    }),
                    config_start.elapsed().as_millis() as u64,
                ));
                cfg
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::User,
                    format!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'"),
                    config_start.elapsed().as_millis() as u64,
                ));
                return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
            }
        },
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                e.to_string(),
                config_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    };

    // Pre-check: reinfer field names exist
    for name in reinfer {
        if !config.fields.field.iter().any(|f| f.name == *name) {
            return CommandResult::failed(
                std::mem::take(&mut steps),
                ErrorKind::User,
                format!("field '{name}' is not in mdvs.toml"),
                start,
            );
        }
    }

    // 2. Scan — ScannedFiles::scan() directly
    let scan_start = Instant::now();
    let scanned = match ScannedFiles::scan(path, &config.scan) {
        Ok(s) => {
            steps.push(StepEntry::ok(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: config.scan.glob.clone(),
                }),
                scan_start.elapsed().as_millis() as u64,
            ));
            s
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                scan_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    };

    // 3. Infer — InferredSchema::infer() is infallible
    let infer_start = Instant::now();
    let schema = InferredSchema::infer(&scanned);
    steps.push(StepEntry::ok(
        Outcome::Infer(InferOutcome {
            fields_inferred: schema.fields.len(),
        }),
        infer_start.elapsed().as_millis() as u64,
    ));

    let total_files = scanned.files.len();

    // --- Field comparison logic (inline, unchanged from original) ---
    let (protected, targets): (Vec<TomlField>, Vec<TomlField>) = if reinfer_all {
        (vec![], config.fields.field.drain(..).collect())
    } else if !reinfer.is_empty() {
        config
            .fields
            .field
            .drain(..)
            .partition(|f| !reinfer.contains(&f.name))
    } else {
        (config.fields.field.drain(..).collect(), vec![])
    };

    let old_fields: HashMap<&str, &TomlField> =
        targets.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut new_fields: Vec<TomlField> = protected.clone();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = protected.len();

    for inf in &schema.fields {
        if protected.iter().any(|f| f.name == inf.name) {
            continue;
        }
        if config.fields.ignore.contains(&inf.name) {
            continue;
        }

        let new_type = FieldTypeSerde::from(&inf.field_type);
        let toml_field = TomlField {
            name: inf.name.clone(),
            field_type: new_type.clone(),
            allowed: inf.allowed.clone(),
            required: inf.required.clone(),
            nullable: inf.nullable,
        };

        if let Some(old_field) = old_fields.get(inf.name.as_str()) {
            if **old_field == toml_field {
                unchanged += 1;
            } else {
                let mut changes = Vec::new();
                if old_field.field_type != toml_field.field_type {
                    changes.push(FieldChange::Type {
                        old: old_field.field_type.to_string(),
                        new: new_type.to_string(),
                    });
                }
                if old_field.allowed != toml_field.allowed {
                    changes.push(FieldChange::Allowed {
                        old: old_field.allowed.clone(),
                        new: toml_field.allowed.clone(),
                    });
                }
                if old_field.required != toml_field.required {
                    changes.push(FieldChange::Required {
                        old: old_field.required.clone(),
                        new: toml_field.required.clone(),
                    });
                }
                if old_field.nullable != toml_field.nullable {
                    changes.push(FieldChange::Nullable {
                        old: old_field.nullable,
                        new: toml_field.nullable,
                    });
                }
                changed.push(ChangedField {
                    name: inf.name.clone(),
                    changes,
                });
            }
            new_fields.push(toml_field);
        } else {
            // Always collect full detail (verbose=true) — the full outcome carries everything
            added.push(inf.to_discovered(total_files, true));
            new_fields.push(toml_field);
        }
    }

    let mut removed: Vec<RemovedField> = old_fields
        .iter()
        .filter(|(name, _)| !schema.fields.iter().any(|f| f.name == **name))
        .map(|(name, old_field)| RemovedField {
            name: name.to_string(),
            // Always collect full detail
            allowed: Some(old_field.allowed.clone()),
        })
        .collect();
    removed.sort_by(|a, b| a.name.cmp(&b.name));

    info!(
        added = added.len(),
        changed = changed.len(),
        removed = removed.len(),
        "update complete"
    );

    let has_changes = !added.is_empty() || !changed.is_empty() || !removed.is_empty();

    // 4. Write config (Skipped if dry_run or no changes)
    if dry_run || !has_changes {
        steps.push(StepEntry::skipped());
    } else {
        let write_start = Instant::now();
        let write_path = path.join("mdvs.toml");
        config.fields.field = new_fields;

        match config.write(&write_path) {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::WriteConfig(WriteConfigOutcome {
                        config_path: write_path.display().to_string(),
                        fields_written: config.fields.field.len(),
                    }),
                    write_start.elapsed().as_millis() as u64,
                ));
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    e.to_string(),
                    write_start.elapsed().as_millis() as u64,
                ));
                return CommandResult::failed(
                    steps,
                    ErrorKind::Application,
                    "failed to write config".into(),
                    start,
                );
            }
        }
    }

    CommandResult {
        steps,
        result: Ok(Outcome::Update(Box::new(UpdateOutcome {
            files_scanned: total_files,
            added,
            changed,
            removed,
            unchanged,
            dry_run,
        }))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::commands::UpdateOutcome;
    use crate::schema::config::MdvsToml;
    use crate::schema::shared::{FieldTypeSerde, ScanConfig};
    use std::fs;

    fn unwrap_update(result: &CommandResult) -> &UpdateOutcome {
        match &result.result {
            Ok(Outcome::Update(o)) => o,
            other => panic!("expected Ok(Update), got: {other:?}"),
        }
    }

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text.",
        )
        .unwrap();
    }

    fn init_no_build(dir: &Path) {
        let step = crate::cmd::init::run(dir, "**", false, false, true, false, false);
        assert!(!crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let step = run(tmp.path(), &[], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert!(result.added.is_empty());
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.unchanged, 3); // title, tags, draft
    }

    #[tokio::test]
    async fn new_fields_discovered() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let step = run(tmp.path(), &[], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].name, "author");
        assert_eq!(result.added[0].field_type, "String");
        assert_eq!(result.added[0].files_found, 1);
        assert_eq!(result.added[0].total_files, 3);
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.unchanged, 3);

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "author"));
    }

    #[tokio::test]
    async fn reinfer_changes_type() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags: single-tag\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let step = run(tmp.path(), &["tags".to_string()], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "tags");
        assert!(result.changed[0]
            .changes
            .iter()
            .any(|c| matches!(c, FieldChange::Type { new, .. } if new == "String")));
    }

    #[tokio::test]
    async fn reinfer_removes_disappeared() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let step = run(tmp.path(), &["tags".to_string()], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert_eq!(result.removed.len(), 1);
        assert_eq!(result.removed[0].name, "tags");

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(!toml.fields.field.iter().any(|f| f.name == "tags"));
    }

    #[tokio::test]
    async fn reinfer_unknown_field_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let step = run(
            tmp.path(),
            &["nonexistent".to_string()],
            false,
            false,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn reinfer_and_reinfer_all_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let step = run(tmp.path(), &["tags".to_string()], true, false, false).await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), &[], true, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert_eq!(result.unchanged, 3);
        assert!(result.added.is_empty());
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());

        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before.scan, toml_after.scan);
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let toml_before = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), &[], false, true, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert!(result.dry_run);
        assert_eq!(result.added.len(), 1);

        let toml_after = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before, toml_after);
    }

    #[tokio::test]
    async fn build_override_false() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let step = run(tmp.path(), &[], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn reinfer_all_detects_glob_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\n---\n# Hello\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\n---\n# World\nMore.",
        )
        .unwrap();
        fs::write(blog_dir.join("bare.md"), "# No frontmatter\nJust content.").unwrap();

        let step = crate::cmd::init::run(tmp.path(), "**", false, false, true, false, false);
        assert!(!crate::step::has_failed(&step));

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_before = toml_before
            .fields
            .field
            .iter()
            .find(|f| f.name == "title")
            .unwrap();
        assert_eq!(title_before.required, vec!["**"]);

        let mut config = toml_before;
        config.scan.include_bare_files = true;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), &[], true, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);
        assert!(
            !result.added.is_empty() || !result.changed.is_empty() || !result.removed.is_empty()
        );

        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_after = toml_after
            .fields
            .field
            .iter()
            .find(|f| f.name == "title")
            .unwrap();
        assert!(!title_after.required.contains(&"**".to_string()));
    }

    #[tokio::test]
    async fn disappearing_field_stays_in_default_mode() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let step = run(tmp.path(), &[], false, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);
        assert!(result.added.is_empty() && result.changed.is_empty() && result.removed.is_empty());

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "tags"));
    }
}
