use crate::discover::infer::InferredSchema;
use crate::discover::infer::constraints::infer_constraints;
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

/// Arguments for the `update reinfer` subcommand.
#[derive(Debug, Clone, clap::Args)]
pub struct ReinferArgs {
    /// Fields to reinfer (all if none specified)
    pub fields: Vec<String>,
    /// Force categorical on named fields (skip heuristic)
    #[arg(long)]
    pub categorical: bool,
    /// Force NOT categorical on named fields (strip categories)
    #[arg(long, conflicts_with = "categorical")]
    pub no_categorical: bool,
    /// Max distinct values for categorical inference
    #[arg(long)]
    pub max_categories: Option<usize>,
    /// Min average repetition for categorical inference
    #[arg(long)]
    pub min_repetition: Option<usize>,
    /// Show what would change, write nothing
    #[arg(long)]
    pub dry_run: bool,
}

/// Re-scan files, infer field changes, and update `mdvs.toml`.
/// Pure inference — no build step.
#[instrument(name = "update", skip_all)]
pub async fn run(
    path: &Path,
    reinfer: Option<&ReinferArgs>,
    dry_run: bool,
    _verbose: bool,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // Pre-check: --categorical/--no-categorical require named fields
    if let Some(args) = reinfer
        && args.fields.is_empty()
        && (args.categorical || args.no_categorical)
    {
        return CommandResult::failed(
            steps,
            ErrorKind::User,
            "--categorical and --no-categorical require named fields".into(),
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
    if let Some(args) = reinfer {
        for name in &args.fields {
            if !config.fields.field.iter().any(|f| f.name == *name) {
                return CommandResult::failed(
                    std::mem::take(&mut steps),
                    ErrorKind::User,
                    format!("field '{name}' is not in mdvs.toml"),
                    start,
                );
            }
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

    // --- Field comparison logic ---
    let reinfer_all = reinfer.is_some_and(|a| a.fields.is_empty());
    let reinfer_fields: Vec<String> = reinfer.map(|a| a.fields.clone()).unwrap_or_default();

    let (protected, targets): (Vec<TomlField>, Vec<TomlField>) = if reinfer_all {
        (vec![], config.fields.field.drain(..).collect())
    } else if !reinfer_fields.is_empty() {
        config
            .fields
            .field
            .drain(..)
            .partition(|f| !reinfer_fields.contains(&f.name))
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
        let constraints = if let Some(args) = reinfer {
            if args.no_categorical {
                None
            } else if args.categorical {
                force_categorical(inf)
            } else {
                let max_cat = args.max_categories.unwrap_or(config.fields.max_categories);
                let min_rep = args
                    .min_repetition
                    .unwrap_or(config.fields.min_category_repetition);
                infer_constraints(inf, max_cat, min_rep)
            }
        } else {
            None
        };
        let toml_field = TomlField {
            name: inf.name.clone(),
            field_type: new_type.clone(),
            allowed: inf.allowed.clone(),
            required: inf.required.clone(),
            nullable: inf.nullable,
            constraints,
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

/// Force categorical constraints on a field: collect all distinct values as categories
/// without applying the heuristic. Only applicable types get categories.
fn force_categorical(
    field: &crate::discover::infer::InferredField,
) -> Option<crate::schema::constraints::Constraints> {
    use crate::discover::field_type::FieldType;
    use crate::schema::constraints::Constraints;

    let applicable = match &field.field_type {
        FieldType::String | FieldType::Integer => true,
        FieldType::Array(inner) => {
            matches!(inner.as_ref(), FieldType::String | FieldType::Integer)
        }
        _ => false,
    };

    if !applicable || field.distinct_values.is_empty() {
        return None;
    }

    let mut categories: Vec<toml::Value> = field
        .distinct_values
        .iter()
        .filter_map(|v| match v {
            serde_json::Value::String(s) => Some(toml::Value::String(s.clone())),
            serde_json::Value::Number(n) => n.as_i64().map(toml::Value::Integer),
            _ => None,
        })
        .collect();

    categories.sort_by(|a, b| match (a, b) {
        (toml::Value::String(a), toml::Value::String(b)) => a.cmp(b),
        (toml::Value::Integer(a), toml::Value::Integer(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    });

    Some(Constraints {
        categories: Some(categories),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::commands::UpdateOutcome;
    use crate::schema::config::MdvsToml;
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

    fn reinfer_args(fields: &[&str]) -> ReinferArgs {
        ReinferArgs {
            fields: fields.iter().map(|s| s.to_string()).collect(),
            categorical: false,
            no_categorical: false,
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        }
    }

    #[tokio::test]
    async fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let step = run(tmp.path(), None, false, false).await;
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

        let step = run(tmp.path(), None, false, false).await;
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

        let step = run(tmp.path(), Some(&reinfer_args(&["tags"])), false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);

        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "tags");
        assert!(
            result.changed[0]
                .changes
                .iter()
                .any(|c| matches!(c, FieldChange::Type { new, .. } if new == "String"))
        );
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

        let step = run(tmp.path(), Some(&reinfer_args(&["tags"])), false, false).await;
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
            Some(&reinfer_args(&["nonexistent"])),
            false,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), Some(&reinfer_args(&[])), false, false).await;
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

        let step = run(tmp.path(), None, true, false).await;
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

        let step = run(tmp.path(), None, false, false).await;
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

        let step = run(tmp.path(), Some(&reinfer_args(&[])), false, false).await;
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

        let step = run(tmp.path(), None, false, false).await;
        assert!(!crate::step::has_failed(&step));
        let result = unwrap_update(&step);
        assert!(result.added.is_empty() && result.changed.is_empty() && result.removed.is_empty());

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "tags"));
    }

    // -----------------------------------------------------------------------
    // Categorical inference in reinfer
    // -----------------------------------------------------------------------

    fn create_categorical_vault(dir: &Path) {
        let blog = dir.join("blog");
        fs::create_dir_all(&blog).unwrap();
        for (i, status) in [
            "draft",
            "draft",
            "published",
            "published",
            "archived",
            "archived",
        ]
        .iter()
        .enumerate()
        {
            fs::write(
                blog.join(format!("post{i}.md")),
                format!("---\nstatus: {status}\ntitle: Post {i}\n---\nBody."),
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn reinfer_infers_categories() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // Init should have inferred categories on status (3 distinct, 6 files, ratio=2)
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status.constraints.is_some());

        // Reinfer status — should re-infer categories
        let step = run(tmp.path(), Some(&reinfer_args(&["status"])), false, false).await;
        assert!(!crate::step::has_failed(&step));

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        let cats = status
            .constraints
            .as_ref()
            .unwrap()
            .categories
            .as_ref()
            .unwrap();
        assert_eq!(cats.len(), 3);
    }

    #[tokio::test]
    async fn reinfer_no_categorical_strips() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec!["status".into()],
            categorical: false,
            no_categorical: true,
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false, false).await;
        assert!(!crate::step::has_failed(&step));

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status.constraints.is_none());
    }

    #[tokio::test]
    async fn reinfer_categorical_forces() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // title has 6 distinct values across 6 files — ratio=1, below threshold
        // But --categorical should force it
        let args = ReinferArgs {
            fields: vec!["title".into()],
            categorical: true,
            no_categorical: false,
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false, false).await;
        assert!(!crate::step::has_failed(&step));

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "title")
            .unwrap();
        let cats = title
            .constraints
            .as_ref()
            .unwrap()
            .categories
            .as_ref()
            .unwrap();
        assert_eq!(cats.len(), 6);
    }

    #[tokio::test]
    async fn reinfer_threshold_override() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // status has 3 distinct, 6 occurrences → ratio 2
        // Set min_repetition=3 → should NOT be categorical
        let args = ReinferArgs {
            fields: vec!["status".into()],
            categorical: false,
            no_categorical: false,
            max_categories: None,
            min_repetition: Some(3),
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false, false).await;
        assert!(!crate::step::has_failed(&step));

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status.constraints.is_none());
    }

    #[tokio::test]
    async fn categorical_without_fields_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec![],
            categorical: true,
            no_categorical: false,
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false, false).await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn init_then_reinfer_then_check_passes() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // Reinfer status
        let step = run(tmp.path(), Some(&reinfer_args(&["status"])), false, false).await;
        assert!(!crate::step::has_failed(&step));

        // Check should still pass after reinfer
        let check_step = crate::cmd::check::run(tmp.path(), true, false);
        let check_result = match &check_step.result {
            Ok(crate::outcome::Outcome::Check(o)) => o,
            other => panic!("expected Ok(Check), got: {other:?}"),
        };
        assert!(check_result.violations.is_empty());
    }

    #[tokio::test]
    async fn init_then_reinfer_all_preserves_categories() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // Verify categories exist after init
        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status_before = toml_before
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status_before.constraints.is_some());

        // Reinfer all
        let step = run(tmp.path(), Some(&reinfer_args(&[])), false, false).await;
        assert!(!crate::step::has_failed(&step));

        // Categories should still be present on status
        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status_after = toml_after
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status_after.constraints.is_some());
        let cats = status_after
            .constraints
            .as_ref()
            .unwrap()
            .categories
            .as_ref()
            .unwrap();
        assert_eq!(cats.len(), 3);
    }
}
