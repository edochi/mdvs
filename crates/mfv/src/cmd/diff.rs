use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::process;

use anyhow::{Context, Result, bail};

use mdvs_schema::lock::LockField;
use mdvs_schema::{FieldType, LockFile, Schema, discover_fields};
use crate::report::{OutputFormat, format_diagnostics, validate};
use crate::scan::scan_directory;

use super::{lock_path_for, resolve_schema_path};

/// Compare current directory state against the lock file and report changes.
pub fn cmd_diff(dir: &Path, config_arg: Option<&Path>, ignore_validation_errors: bool) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }

    let config_path = resolve_schema_path(dir, config_arg)?;

    let schema = Schema::from_file(&config_path)
        .with_context(|| format!("failed to load config from {}", config_path.display()))?;

    let lock_path = lock_path_for(&config_path);
    if !lock_path.exists() {
        bail!(
            "no lock file found at {} — run `mfv init` or `mfv update` first",
            lock_path.display()
        );
    }

    let old_lock = LockFile::from_file(&lock_path)
        .with_context(|| format!("failed to load lock from {}", lock_path.display()))?;

    eprintln!("Scanning {} with glob '{}'...", dir.display(), &schema.glob);
    let all_files = scan_directory(dir, &schema.glob, schema.frontmatter_format)?;

    if all_files.is_empty() {
        bail!("no markdown files found matching '{}'", &schema.glob);
    }

    let files: Vec<_> = if !schema.include_bare_files {
        all_files
            .into_iter()
            .filter(|f| f.frontmatter.is_some())
            .collect()
    } else {
        all_files
    };
    let total = files.len();
    eprintln!("{} markdown files considered\n", total);

    // Validation gate
    let diagnostics = validate(&files, &schema);
    if !diagnostics.is_empty() {
        eprint!(
            "{}",
            format_diagnostics(&diagnostics, OutputFormat::Human)
        );
        if !ignore_validation_errors {
            bail!(
                "{} validation error(s) — use --ignore-validation-errors to diff anyway",
                diagnostics.len()
            );
        }
        eprintln!("Ignoring {} validation error(s), continuing diff...\n", diagnostics.len());
    }

    // Build new lock from current state
    let file_frontmatters: Vec<(&str, Option<&serde_json::Value>)> = files
        .iter()
        .map(|f| (f.rel_path.as_str(), f.frontmatter.as_ref()))
        .collect();
    let files_with_frontmatter = file_frontmatters
        .iter()
        .filter(|(_, fm)| fm.is_some())
        .count();
    let field_infos = discover_fields(&file_frontmatters);

    let generated_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
    let new_lock = LockFile::from_discovery(
        &field_infos,
        total,
        files_with_frontmatter,
        &schema.glob,
        &generated_at,
    );

    let diff = diff_locks(&old_lock, &new_lock);

    if diff.is_empty() {
        println!("No changes detected.");
        process::exit(0);
    }

    print!("{}", format_diff(&diff, &old_lock, &new_lock));
    process::exit(1);
}

// --- Lock diffing logic (absorbed from src/diff.rs) ---

/// A type change between old and new lock.
#[derive(Debug)]
struct TypeChange {
    /// Field name.
    name: String,
    /// Type in the old lock.
    old_type: FieldType,
    /// Type in the new lock.
    new_type: FieldType,
}

/// A file coverage change for a field.
#[derive(Debug)]
struct CoverageChange {
    /// Field name.
    name: String,
    /// Files that gained this field.
    added_files: Vec<String>,
    /// Files that lost this field.
    removed_files: Vec<String>,
}

/// The result of comparing two lock files.
#[derive(Debug)]
struct LockDiff {
    /// Fields present in new but not old.
    added: Vec<LockField>,
    /// Fields present in old but not new.
    removed: Vec<LockField>,
    /// Fields with a different inferred type.
    type_changed: Vec<TypeChange>,
    /// Fields with changed file coverage (same type).
    coverage_changed: Vec<CoverageChange>,
}

impl LockDiff {
    /// Returns true if there are no changes.
    fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.type_changed.is_empty()
            && self.coverage_changed.is_empty()
    }
}

/// Compare two lock files and return the differences.
fn diff_locks(
    old: &LockFile,
    new: &LockFile,
) -> LockDiff {
    let old_map: HashMap<&str, &LockField> =
        old.fields.iter().map(|f| (f.name.as_str(), f)).collect();
    let new_map: HashMap<&str, &LockField> =
        new.fields.iter().map(|f| (f.name.as_str(), f)).collect();

    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut type_changed = Vec::new();
    let mut coverage_changed = Vec::new();

    // Fields in new but not old → added
    for field in &new.fields {
        if !old_map.contains_key(field.name.as_str()) {
            added.push(LockField {
                name: field.name.clone(),
                field_type: field.field_type.clone(),
                files: field.files.clone(),
            });
        }
    }

    // Fields in old but not new → removed
    for field in &old.fields {
        if !new_map.contains_key(field.name.as_str()) {
            removed.push(LockField {
                name: field.name.clone(),
                field_type: field.field_type.clone(),
                files: field.files.clone(),
            });
        }
    }

    // Fields in both → check for type or coverage changes
    for field in &old.fields {
        if let Some(new_field) = new_map.get(field.name.as_str()) {
            if field.field_type != new_field.field_type {
                type_changed.push(TypeChange {
                    name: field.name.clone(),
                    old_type: field.field_type.clone(),
                    new_type: new_field.field_type.clone(),
                });
            } else {
                let old_set: BTreeSet<&str> =
                    field.files.iter().map(|s| s.as_str()).collect();
                let new_set: BTreeSet<&str> =
                    new_field.files.iter().map(|s| s.as_str()).collect();

                let added_files: Vec<String> = new_set
                    .difference(&old_set)
                    .map(|s| s.to_string())
                    .collect();
                let removed_files: Vec<String> = old_set
                    .difference(&new_set)
                    .map(|s| s.to_string())
                    .collect();

                if !added_files.is_empty() || !removed_files.is_empty() {
                    coverage_changed.push(CoverageChange {
                        name: field.name.clone(),
                        added_files,
                        removed_files,
                    });
                }
            }
        }
    }

    LockDiff {
        added,
        removed,
        type_changed,
        coverage_changed,
    }
}

/// Format a diff as a human-readable report.
fn format_diff(
    diff: &LockDiff,
    old: &LockFile,
    new: &LockFile,
) -> String {
    let mut out = String::new();

    if diff.is_empty() {
        out.push_str("No changes detected.\n");
        return out;
    }

    if !diff.added.is_empty() {
        out.push_str(&format!("Fields added ({}):\n", diff.added.len()));
        for f in &diff.added {
            out.push_str(&format!(
                "  + {} ({}) — {} file(s)\n",
                f.name,
                f.field_type,
                f.files.len()
            ));
        }
        out.push('\n');
    }

    if !diff.removed.is_empty() {
        out.push_str(&format!("Fields removed ({}):\n", diff.removed.len()));
        for f in &diff.removed {
            out.push_str(&format!(
                "  - {} ({}) — was in {} file(s)\n",
                f.name,
                f.field_type,
                f.files.len()
            ));
        }
        out.push('\n');
    }

    if !diff.type_changed.is_empty() {
        out.push_str(&format!("Type changes ({}):\n", diff.type_changed.len()));
        for tc in &diff.type_changed {
            out.push_str(&format!(
                "  ~ {}: {} → {}\n",
                tc.name, tc.old_type, tc.new_type
            ));
        }
        out.push('\n');
    }

    if !diff.coverage_changed.is_empty() {
        out.push_str(&format!(
            "Coverage changes ({}):\n",
            diff.coverage_changed.len()
        ));
        for cc in &diff.coverage_changed {
            let added = if cc.added_files.is_empty() {
                String::new()
            } else {
                format!("+{}", cc.added_files.len())
            };
            let removed = if cc.removed_files.is_empty() {
                String::new()
            } else {
                format!("-{}", cc.removed_files.len())
            };
            let parts: Vec<&str> = [added.as_str(), removed.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect();
            out.push_str(&format!("  ~ {}: {} file(s)\n", cc.name, parts.join(", ")));
            for f in &cc.added_files {
                out.push_str(&format!("    + {f}\n"));
            }
            for f in &cc.removed_files {
                out.push_str(&format!("    - {f}\n"));
            }
        }
        out.push('\n');
    }

    out.push_str(&format!(
        "Summary: {} files (was {}), {} fields (was {})\n",
        new.discovery.total_files,
        old.discovery.total_files,
        new.fields.len(),
        old.fields.len()
    ));

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mdvs_schema::LockFile;
    use mdvs_schema::lock::LockDiscovery;

    fn make_lock(fields: Vec<LockField>) -> LockFile {
        LockFile {
            discovery: LockDiscovery {
                total_files: 10,
                files_with_frontmatter: 8,
                glob: "**".to_string(),
                generated_at: "2025-01-01T00:00:00".to_string(),
            },
            fields,
        }
    }

    fn make_field(name: &str, ft: FieldType, files: &[&str]) -> LockField {
        LockField {
            name: name.to_string(),
            field_type: ft,
            files: files.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn no_changes() {
        let old = make_lock(vec![
            make_field("title", FieldType::String, &["a.md", "b.md"]),
        ]);
        let new = make_lock(vec![
            make_field("title", FieldType::String, &["a.md", "b.md"]),
        ]);
        let diff = diff_locks(&old, &new);
        assert!(diff.is_empty());
    }

    #[test]
    fn added_field() {
        let old = make_lock(vec![
            make_field("title", FieldType::String, &["a.md"]),
        ]);
        let new = make_lock(vec![
            make_field("title", FieldType::String, &["a.md"]),
            make_field("tags", FieldType::StringArray, &["a.md", "b.md"]),
        ]);
        let diff = diff_locks(&old, &new);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].name, "tags");
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn removed_field() {
        let old = make_lock(vec![
            make_field("title", FieldType::String, &["a.md"]),
            make_field("draft", FieldType::Boolean, &["a.md"]),
        ]);
        let new = make_lock(vec![
            make_field("title", FieldType::String, &["a.md"]),
        ]);
        let diff = diff_locks(&old, &new);
        assert!(diff.added.is_empty());
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0].name, "draft");
    }

    #[test]
    fn type_change() {
        let old = make_lock(vec![
            make_field("count", FieldType::Integer, &["a.md"]),
        ]);
        let new = make_lock(vec![
            make_field("count", FieldType::Float, &["a.md"]),
        ]);
        let diff = diff_locks(&old, &new);
        assert_eq!(diff.type_changed.len(), 1);
        assert_eq!(diff.type_changed[0].name, "count");
        assert_eq!(diff.type_changed[0].old_type, FieldType::Integer);
        assert_eq!(diff.type_changed[0].new_type, FieldType::Float);
    }

    #[test]
    fn coverage_change() {
        let old = make_lock(vec![
            make_field("title", FieldType::String, &["a.md", "b.md"]),
        ]);
        let new = make_lock(vec![
            make_field("title", FieldType::String, &["a.md", "c.md"]),
        ]);
        let diff = diff_locks(&old, &new);
        assert!(diff.added.is_empty());
        assert!(diff.type_changed.is_empty());
        assert_eq!(diff.coverage_changed.len(), 1);
        assert_eq!(diff.coverage_changed[0].name, "title");
        assert_eq!(diff.coverage_changed[0].added_files, vec!["c.md"]);
        assert_eq!(diff.coverage_changed[0].removed_files, vec!["b.md"]);
    }
}
