use crate::cmd::steps::{infer_step, read_config_step, scan_step};
use crate::discover::infer::constraints::{infer_constraints, infer_range};
use crate::discover::infer::{InferredField, InferredSchema};
use crate::outcome::commands::UpdateOutcome;
use crate::outcome::{Outcome, WriteConfigOutcome};
use crate::output::{ChangedField, FieldChange, RemovedField};
use crate::schema::config::{FieldsConfig, TomlField};
use crate::schema::constraints::Constraints;
use crate::schema::shared::FieldTypeSerde;
use crate::step::{CommandResult, ErrorKind, StepEntry, elapsed_ms};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::{info, instrument};

/// Constraint kinds selectable via `--with`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[value(rename_all = "lowercase")]
pub enum WithKind {
    /// Force categorical inference (skip heuristic)
    Categorical,
    /// Infer min/max range from observed numeric values
    Range,
    /// Strip all constraints (no auto-inference)
    None,
}

/// Whether two `WithKind` values conflict on the same field.
fn with_kinds_conflict(a: WithKind, b: WithKind) -> bool {
    use WithKind::{Categorical, Range};
    matches!((a, b), (Categorical, Range) | (Range, Categorical))
}

/// Arguments for the `update reinfer` subcommand.
#[derive(Debug, Clone, clap::Args)]
pub struct ReinferArgs {
    /// Fields to reinfer (all if none specified)
    pub fields: Vec<String>,
    /// Which constraint kinds to (re)infer on named fields.
    /// Comma-separated list. Use `none` to strip all constraints.
    /// Omit entirely to run heuristic defaults.
    #[arg(long = "with", value_delimiter = ',', value_enum)]
    pub with: Vec<WithKind>,
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
pub async fn run(path: &Path, reinfer: Option<&ReinferArgs>, dry_run: bool) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // Pre-check: --with requires named fields, validate the kind list
    if let Some(args) = reinfer
        && let Err(msg) = validate_with_args(args)
    {
        return CommandResult::failed(steps, ErrorKind::User, msg, start);
    }

    let Ok((mut config, config_path)) = read_config_step(path, &mut steps) else {
        return CommandResult::failed_from_steps(steps, start);
    };

    // Pre-check: reinfer field names exist
    if let Some(args) = reinfer {
        for name in &args.fields {
            if !config.fields.field.iter().any(|f| f.name == *name) {
                return CommandResult::failed(
                    steps,
                    ErrorKind::User,
                    format!("field '{name}' is not in mdvs.toml"),
                    start,
                );
            }
        }
    }

    let Ok(scanned) = scan_step(path, &config.scan, &mut steps) else {
        return CommandResult::failed_from_steps(steps, start);
    };

    let schema = infer_step(&scanned, &mut steps);

    let existing = std::mem::take(&mut config.fields.field);
    let (new_fields, outcome) = plan_update(
        existing,
        &config.fields,
        &schema,
        reinfer,
        scanned.files.len(),
        dry_run,
    );

    info!(
        added = outcome.added.len(),
        changed = outcome.changed.len(),
        removed = outcome.removed.len(),
        "update complete"
    );

    // Write config (Skipped if dry_run or no changes)
    if dry_run || !outcome.has_changes() {
        steps.push(StepEntry::skipped());
    } else {
        let write_start = Instant::now();
        config.fields.field = new_fields;

        match config.write(&config_path) {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::WriteConfig(WriteConfigOutcome {
                        config_path: config_path.display().to_string(),
                        fields_written: config.fields.field.len(),
                    }),
                    elapsed_ms(write_start),
                ));
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    e.to_string(),
                    elapsed_ms(write_start),
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
        result: Ok(Outcome::Update(Box::new(outcome))),
        elapsed_ms: elapsed_ms(start),
    }
}

/// Check the `--with` list of a reinfer invocation for usage errors.
///
/// Returns the user-facing message for the first problem found.
fn validate_with_args(args: &ReinferArgs) -> Result<(), String> {
    if !args.with.is_empty() && args.fields.is_empty() {
        return Err("--with requires named fields".into());
    }
    if args.with.contains(&WithKind::None) && args.with.len() > 1 {
        return Err("--with=none cannot be combined with other kinds".into());
    }
    for (i, &a) in args.with.iter().enumerate() {
        for &b in &args.with[i + 1..] {
            if with_kinds_conflict(a, b) {
                return Err(format!("--with: {a:?} and {b:?} are mutually exclusive"));
            }
        }
    }
    Ok(())
}

/// Compare the inferred schema against the existing field definitions.
///
/// Returns the full field list to write (protected fields followed by
/// inferred ones) and the outcome describing the difference. Fields named
/// for reinfer (or all of them for a bare reinfer) are targets that
/// inference may change or remove; the rest are protected and kept as-is.
/// Without reinfer, every existing field is protected and only new fields
/// are added. `fields` supplies the ignore list and categorical thresholds.
fn plan_update(
    existing: Vec<TomlField>,
    fields: &FieldsConfig,
    schema: &InferredSchema,
    reinfer: Option<&ReinferArgs>,
    files_scanned: usize,
    dry_run: bool,
) -> (Vec<TomlField>, UpdateOutcome) {
    let reinfer_all = reinfer.is_some_and(|a| a.fields.is_empty());
    let reinfer_fields: Vec<String> = reinfer.map(|a| a.fields.clone()).unwrap_or_default();

    let (protected, targets): (Vec<TomlField>, Vec<TomlField>) = if reinfer_all {
        (vec![], existing)
    } else if !reinfer_fields.is_empty() {
        existing
            .into_iter()
            .partition(|f| !reinfer_fields.contains(&f.name))
    } else {
        (existing, vec![])
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
        if fields.ignore.contains(&inf.name) {
            continue;
        }
        inf.emit_inexact_widening_warning();

        let toml_field = TomlField {
            name: inf.name.clone(),
            field_type: FieldTypeSerde::from(&inf.field_type),
            allowed: inf.allowed.clone(),
            required: inf.required.clone(),
            nullable: inf.nullable,
            constraints: reinfer.and_then(|args| constraints_for(args, inf, fields)),
            preprocess: inf.preprocess.clone(),
        };

        if let Some(old_field) = old_fields.get(inf.name.as_str()) {
            if **old_field == toml_field {
                unchanged += 1;
            } else {
                changed.push(ChangedField {
                    name: inf.name.clone(),
                    changes: diff_field(old_field, &toml_field),
                });
            }
        } else {
            // Always collect full detail (verbose=true) — the full outcome carries everything
            added.push(inf.to_discovered(files_scanned, true));
        }
        new_fields.push(toml_field);
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

    let outcome = UpdateOutcome {
        files_scanned,
        added,
        changed,
        removed,
        unchanged,
        dry_run,
    };
    (new_fields, outcome)
}

/// Constraints for a reinferred field, according to `--with`.
///
/// `none` strips constraints; no `--with` runs the categorical heuristic
/// (with the command-line overrides falling back to the config's
/// thresholds); explicit kinds force-infer each one.
fn constraints_for(
    args: &ReinferArgs,
    inf: &InferredField,
    fields: &FieldsConfig,
) -> Option<Constraints> {
    if args.with.contains(&WithKind::None) {
        return None;
    }
    if args.with.is_empty() {
        let max_cat = args.max_categories.unwrap_or(fields.max_categories());
        let min_rep = args
            .min_repetition
            .unwrap_or(fields.min_category_repetition());
        return infer_constraints(inf, max_cat, min_rep);
    }
    let mut c = Constraints::default();
    for kind in &args.with {
        match kind {
            WithKind::Categorical => {
                if let Some(forced) = force_categorical(inf) {
                    c.categories = forced.categories;
                }
            }
            WithKind::Range => {
                if let Some(r) = infer_range(inf) {
                    c.min = r.min;
                    c.max = r.max;
                }
            }
            // `None` is handled by the early return above. If a future
            // caller bypasses that, fall through silently rather than panic.
            WithKind::None => {}
        }
    }
    (c != Constraints::default()).then_some(c)
}

/// The per-attribute changes between an existing field and its reinferred
/// definition. Constraint and preprocess changes are not itemized.
fn diff_field(old: &TomlField, new: &TomlField) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    if old.field_type != new.field_type {
        changes.push(FieldChange::Type {
            old: old.field_type.to_string(),
            new: new.field_type.to_string(),
        });
    }
    if old.allowed != new.allowed {
        changes.push(FieldChange::Allowed {
            old: old.allowed.clone(),
            new: new.allowed.clone(),
        });
    }
    if old.required != new.required {
        changes.push(FieldChange::Required {
            old: old.required.clone(),
            new: new.required.clone(),
        });
    }
    if old.nullable != new.nullable {
        changes.push(FieldChange::Nullable {
            old: old.nullable,
            new: new.nullable,
        });
    }
    changes
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
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::init::{InitOptions, InitScanFlags};
    use crate::discover::field_type::FieldType;
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
        let step = crate::cmd::init::run(
            dir,
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
        assert!(!crate::step::has_failed(&step));
    }

    fn reinfer_args(fields: &[&str]) -> ReinferArgs {
        ReinferArgs {
            fields: fields.iter().map(ToString::to_string).collect(),
            with: vec![],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        }
    }

    fn with_args(fields: &[&str], with: &[WithKind]) -> ReinferArgs {
        ReinferArgs {
            with: with.to_vec(),
            ..reinfer_args(fields)
        }
    }

    #[test]
    fn validate_with_args_accepts_valid_lists() {
        assert_eq!(validate_with_args(&with_args(&[], &[])), Ok(()));
        assert_eq!(
            validate_with_args(&with_args(&["a"], &[WithKind::Categorical])),
            Ok(())
        );
    }

    #[test]
    fn validate_with_args_requires_named_fields() {
        assert_eq!(
            validate_with_args(&with_args(&[], &[WithKind::Range])),
            Err("--with requires named fields".to_string())
        );
    }

    #[test]
    fn validate_with_args_rejects_none_with_others() {
        assert_eq!(
            validate_with_args(&with_args(&["a"], &[WithKind::None, WithKind::Range])),
            Err("--with=none cannot be combined with other kinds".to_string())
        );
    }

    #[test]
    fn validate_with_args_reports_first_conflict_in_order() {
        assert_eq!(
            validate_with_args(&with_args(
                &["a"],
                &[WithKind::Range, WithKind::Range, WithKind::Categorical]
            )),
            Err("--with: Range and Categorical are mutually exclusive".to_string())
        );
        assert_eq!(
            validate_with_args(&with_args(
                &["a"],
                &[WithKind::Categorical, WithKind::Range]
            )),
            Err("--with: Categorical and Range are mutually exclusive".to_string())
        );
    }

    fn string_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    #[test]
    fn diff_field_identical_is_empty() {
        let f = string_field("title");
        assert!(diff_field(&f, &f).is_empty());
    }

    #[test]
    fn diff_field_reports_type_change() {
        let old = string_field("n");
        let new = TomlField {
            field_type: FieldTypeSerde::Scalar("Integer".into()),
            ..string_field("n")
        };
        assert_eq!(
            serde_json::to_value(diff_field(&old, &new)).unwrap(),
            serde_json::json!([{"aspect": "type", "old": "String", "new": "Integer"}])
        );
    }

    #[test]
    fn diff_field_reports_allowed_and_required_changes_in_order() {
        let old = string_field("t");
        let new = TomlField {
            allowed: vec!["blog/**".into()],
            required: vec!["blog/**".into()],
            ..string_field("t")
        };
        assert_eq!(
            serde_json::to_value(diff_field(&old, &new)).unwrap(),
            serde_json::json!([
                {"aspect": "allowed", "old": ["**"], "new": ["blog/**"]},
                {"aspect": "required", "old": [], "new": ["blog/**"]},
            ])
        );
    }

    /// A String field seen in `occurrences` files with the given distinct values.
    fn inferred_string(values: &[&str], occurrences: usize) -> InferredField {
        InferredField {
            name: "status".into(),
            field_type: FieldType::String,
            files: vec![],
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            distinct_values: values.iter().map(|v| serde_json::json!(v)).collect(),
            occurrence_count: occurrences,
            preprocess: vec![],
        }
    }

    fn default_fields_config() -> FieldsConfig {
        FieldsConfig {
            ignore: vec![],
            field: vec![],
            max_categories: None,
            min_category_repetition: None,
        }
    }

    /// Occurrences per distinct value, enough for the categorical heuristic.
    const REPEATED_OCCURRENCES: usize = 20;

    #[test]
    fn constraints_for_none_strips_constraints() {
        let inf = inferred_string(&["draft", "done"], REPEATED_OCCURRENCES);
        let args = with_args(&["status"], &[WithKind::None]);
        assert_eq!(constraints_for(&args, &inf, &default_fields_config()), None);
    }

    #[test]
    fn constraints_for_empty_with_runs_heuristic() {
        let inf = inferred_string(&["draft", "done"], REPEATED_OCCURRENCES);
        let fields = default_fields_config();
        let expected = infer_constraints(
            &inf,
            fields.max_categories(),
            fields.min_category_repetition(),
        );
        assert!(expected.is_some());
        let args = with_args(&["status"], &[]);
        assert_eq!(constraints_for(&args, &inf, &fields), expected);
    }

    #[test]
    fn constraints_for_explicit_kind_yielding_nothing_is_none() {
        // Range on a String field infers no bounds, leaving the default.
        let inf = inferred_string(&["draft", "done"], REPEATED_OCCURRENCES);
        let args = with_args(&["status"], &[WithKind::Range]);
        assert_eq!(constraints_for(&args, &inf, &default_fields_config()), None);
    }

    #[tokio::test]
    async fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let step = run(tmp.path(), None, false).await;
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

        let step = run(tmp.path(), None, false).await;
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

        let step = run(tmp.path(), Some(&reinfer_args(&["tags"])), false).await;
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

        let step = run(tmp.path(), Some(&reinfer_args(&["tags"])), false).await;
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

        let step = run(tmp.path(), Some(&reinfer_args(&["nonexistent"])), false).await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), Some(&reinfer_args(&[])), false).await;
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

        let step = run(tmp.path(), None, true).await;
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

        let step = run(tmp.path(), None, false).await;
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

        let step = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            None,
            None,
        );
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

        let step = run(tmp.path(), Some(&reinfer_args(&[])), false).await;
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

        let step = run(tmp.path(), None, false).await;
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
            "draft",
            "published",
            "published",
            "published",
            "archived",
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

        // Init should have inferred categories on status (3 distinct, 9 files, ratio=3)
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(status.constraints.is_some());

        // Reinfer status — should re-infer categories
        let step = run(tmp.path(), Some(&reinfer_args(&["status"])), false).await;
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
    async fn reinfer_with_none_strips() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec!["status".into()],
            with: vec![WithKind::None],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
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
    async fn reinfer_with_categorical_forces() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // title has 9 distinct values across 9 files — ratio=1, below threshold
        // But --with=categorical should force it
        let args = ReinferArgs {
            fields: vec!["title".into()],
            with: vec![WithKind::Categorical],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
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
        assert_eq!(cats.len(), 9);
    }

    #[tokio::test]
    async fn reinfer_threshold_override() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // status has 3 distinct, 9 occurrences → ratio 3
        // Set min_repetition=4 → should NOT be categorical
        let args = ReinferArgs {
            fields: vec!["status".into()],
            with: vec![],
            max_categories: None,
            min_repetition: Some(4),
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
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
    async fn with_without_fields_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec![],
            with: vec![WithKind::Categorical],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn init_then_reinfer_then_check_passes() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        // Reinfer status
        let step = run(tmp.path(), Some(&reinfer_args(&["status"])), false).await;
        assert!(!crate::step::has_failed(&step));

        // Check should still pass after reinfer
        let check_step = crate::cmd::check::run(tmp.path(), true, false, None);
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
        let step = run(tmp.path(), Some(&reinfer_args(&[])), false).await;
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

    #[tokio::test]
    async fn reinfer_with_none_plus_other_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec!["status".into()],
            with: vec![WithKind::None, WithKind::Categorical],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
        assert!(crate::step::has_failed(&step));
    }

    #[tokio::test]
    async fn reinfer_with_conflicting_kinds_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_categorical_vault(tmp.path());
        init_no_build(tmp.path());

        let args = ReinferArgs {
            fields: vec!["status".into()],
            with: vec![WithKind::Categorical, WithKind::Range],
            max_categories: None,
            min_repetition: None,
            dry_run: false,
        };
        let step = run(tmp.path(), Some(&args), false).await;
        assert!(crate::step::has_failed(&step));
    }
}
