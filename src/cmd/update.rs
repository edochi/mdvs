use crate::cmd::build::BuildResult;
use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::index::storage::check_reserved_names;
use crate::output::{ChangedField, CommandOutput, DiscoveredField};
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::shared::FieldTypeSerde;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use tracing::{info, instrument};

/// Result of the `update` command: field changes discovered by re-inference.
#[derive(Debug, Serialize)]
pub struct UpdateResult {
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Newly discovered fields not previously in `mdvs.toml`.
    pub added: Vec<DiscoveredField>,
    /// Fields whose type or glob constraints changed during re-inference.
    pub changed: Vec<ChangedField>,
    /// Field names that disappeared from all files during re-inference.
    pub removed: Vec<String>,
    /// Number of fields that remained identical.
    pub unchanged: usize,
    /// Whether a build was triggered after updating.
    pub auto_build: bool,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
    /// Build result, if a build was triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_result: Option<BuildResult>,
}

impl UpdateResult {
    fn has_changes(&self) -> bool {
        !self.added.is_empty() || !self.changed.is_empty() || !self.removed.is_empty()
    }
}

impl CommandOutput for UpdateResult {
    fn format_human(&self) -> String {
        let mut out = String::new();

        if !self.has_changes() {
            out.push_str(&format!(
                "Scanned {} files — no changes\n",
                self.files_scanned
            ));
            return out;
        }

        out.push_str(&format!("Scanned {} files\n", self.files_scanned));

        if !self.added.is_empty() {
            out.push_str(&format!("\nAdded {} field(s):\n", self.added.len()));
            for field in &self.added {
                out.push_str(&format!(
                    "  {}  {}  {}/{}\n",
                    field.name, field.field_type, field.files_found, field.total_files,
                ));
            }
        }

        if !self.changed.is_empty() {
            out.push_str(&format!("\nChanged {} field(s):\n", self.changed.len()));
            for field in &self.changed {
                out.push_str(&format!(
                    "  {}  {} -> {}\n",
                    field.name, field.old_type, field.new_type,
                ));
            }
        }

        if !self.removed.is_empty() {
            out.push_str(&format!("\nRemoved {} field(s):\n", self.removed.len()));
            for name in &self.removed {
                out.push_str(&format!("  {name}  (no longer found)\n"));
            }
        }

        if self.unchanged > 0 {
            out.push_str(&format!("\n{} field(s) unchanged\n", self.unchanged));
        }

        if self.dry_run {
            out.push_str("(dry run, nothing written)\n");
        } else {
            out.push_str("\nUpdated mdvs.toml\n");
        }

        if let Some(ref br) = self.build_result {
            out.push('\n');
            out.push_str(&br.format_human());
        }

        out
    }
}

/// Re-scan files, infer field changes, and update `mdvs.toml`.
#[instrument(name = "update", skip_all)]
pub async fn run(
    path: &Path,
    reinfer: &[String],
    reinfer_all: bool,
    build_flag: Option<bool>,
    dry_run: bool,
) -> anyhow::Result<UpdateResult> {
    let config_path = path.join("mdvs.toml");
    let mut config = MdvsToml::read(&config_path)?;

    // Validate flag combinations
    if !reinfer.is_empty() && reinfer_all {
        anyhow::bail!("cannot use --reinfer and --reinfer-all together");
    }

    // Validate reinfer field names exist in toml
    for name in reinfer {
        if !config.fields.field.iter().any(|f| f.name == *name) {
            anyhow::bail!("field '{name}' is not in mdvs.toml");
        }
    }

    // Scan and infer
    let scanned = ScannedFiles::scan(path, &config.scan);
    let schema = InferredSchema::infer(&scanned);
    let total_files = scanned.files.len();

    // Partition existing fields into protected + targets
    let (protected, targets): (Vec<TomlField>, Vec<TomlField>) = if reinfer_all {
        (vec![], config.fields.field.drain(..).collect())
    } else if !reinfer.is_empty() {
        config
            .fields
            .field
            .drain(..)
            .partition(|f| !reinfer.contains(&f.name))
    } else {
        // Default mode: all existing are protected, no targets
        (config.fields.field.drain(..).collect(), vec![])
    };

    // Build old_fields map from targets for comparison (type + globs)
    let old_fields: HashMap<&str, &TomlField> = targets
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();

    let mut new_fields: Vec<TomlField> = protected.clone();
    let mut added = Vec::new();
    let mut changed = Vec::new();
    let mut unchanged = protected.len();

    // Walk inferred fields
    for inf in &schema.fields {
        // Skip if protected (already in new_fields)
        if protected.iter().any(|f| f.name == inf.name) {
            continue;
        }
        // Skip if in ignore list
        if config.fields.ignore.contains(&inf.name) {
            continue;
        }

        let new_type = FieldTypeSerde::from(&inf.field_type);
        let toml_field = TomlField {
            name: inf.name.clone(),
            field_type: new_type.clone(),
            allowed: inf.allowed.clone(),
            required: inf.required.clone(),
        };

        if let Some(old_field) = old_fields.get(inf.name.as_str()) {
            // Was a target for reinference — compare full field (type + globs)
            if **old_field == toml_field {
                unchanged += 1;
            } else {
                changed.push(ChangedField {
                    name: inf.name.clone(),
                    old_type: old_field.field_type.to_string(),
                    new_type: new_type.to_string(),
                });
            }
            new_fields.push(toml_field);
        } else {
            // Genuinely new field
            added.push(DiscoveredField {
                name: inf.name.clone(),
                field_type: new_type.to_string(),
                files_found: inf.files.len(),
                total_files,
            });
            new_fields.push(toml_field);
        }
    }

    // Removed = target names not found in inferred
    let mut removed: Vec<String> = old_fields
        .keys()
        .filter(|name| !schema.fields.iter().any(|f| f.name == **name))
        .map(|name| name.to_string())
        .collect();
    removed.sort();

    info!(
        added = added.len(),
        changed = changed.len(),
        removed = removed.len(),
        "update complete"
    );

    let should_build = build_flag.unwrap_or(config.update.auto_build);

    let mut result = UpdateResult {
        files_scanned: total_files,
        added,
        changed,
        removed,
        unchanged,
        auto_build: should_build && !dry_run,
        dry_run,
        build_result: None,
    };

    if dry_run || !result.has_changes() {
        return Ok(result);
    }

    // Validate field names don't collide with internal column names
    let field_names: Vec<String> = new_fields.iter().map(|f| f.name.clone()).collect();
    check_reserved_names(&field_names, config.internal_prefix())?;

    // Update fields and write
    config.fields.field = new_fields;
    config.write(&config_path)?;

    if should_build {
        result.build_result =
            Some(crate::cmd::build::run(path, None, None, None, false).await?);
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::MdvsToml;
    use std::fs;

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

    async fn init_no_build(dir: &Path) {
        crate::cmd::init::run(
            dir,
            None,
            None,
            "**",
            false,
            false,
            true, // ignore bare files
            None,
            false, // no auto_build
            false, // skip_gitignore
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let result = run(tmp.path(), &[], false, Some(false), false).await.unwrap();

        assert!(!result.has_changes());
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.unchanged, 3); // title, tags, draft
    }

    #[tokio::test]
    async fn new_fields_discovered() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Add a file with a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let result = run(tmp.path(), &[], false, Some(false), false).await.unwrap();

        assert_eq!(result.added.len(), 1);
        assert_eq!(result.added[0].name, "author");
        assert_eq!(result.added[0].field_type, "String");
        assert_eq!(result.added[0].files_found, 1);
        assert_eq!(result.added[0].total_files, 3);
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());
        assert_eq!(result.unchanged, 3); // title, tags, draft

        // Verify toml was updated
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "author"));
    }

    #[tokio::test]
    async fn reinfer_changes_type() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Replace files so tags becomes a string instead of array
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags: single-tag\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let result = run(
            tmp.path(),
            &["tags".to_string()],
            false,
            Some(false),
            false,
        )
        .await
        .unwrap();

        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "tags");
        assert_eq!(result.changed[0].new_type, "String");
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
    }

    #[tokio::test]
    async fn reinfer_removes_disappeared() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Remove tags from all files
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let result = run(
            tmp.path(),
            &["tags".to_string()],
            false,
            Some(false),
            false,
        )
        .await
        .unwrap();

        assert_eq!(result.removed, vec!["tags"]);
        assert!(result.changed.is_empty());
        assert!(result.added.is_empty());

        // Verify toml no longer has tags
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(!toml.fields.field.iter().any(|f| f.name == "tags"));
    }

    #[tokio::test]
    async fn reinfer_unknown_field_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let result = run(
            tmp.path(),
            &["nonexistent".to_string()],
            false,
            Some(false),
            false,
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("field 'nonexistent' is not in mdvs.toml"));
    }

    #[tokio::test]
    async fn reinfer_and_reinfer_all_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let result = run(
            tmp.path(),
            &["tags".to_string()],
            true, // reinfer_all
            Some(false),
            false,
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot use --reinfer and --reinfer-all together"));
    }

    #[tokio::test]
    async fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), &[], true, Some(false), false).await.unwrap();

        // All fields are re-inferred with same types → unchanged
        assert_eq!(result.unchanged, 3);
        assert!(result.added.is_empty());
        assert!(result.changed.is_empty());
        assert!(result.removed.is_empty());

        // Non-field config is preserved
        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before.scan, toml_after.scan);
        assert_eq!(toml_before.update, toml_after.update);
        assert_eq!(toml_before.embedding_model, toml_after.embedding_model);
        assert_eq!(toml_before.chunking, toml_after.chunking);
        assert_eq!(toml_before.search, toml_after.search);
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let toml_before = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), &[], false, Some(false), true).await.unwrap();

        assert!(result.dry_run);
        assert_eq!(result.added.len(), 1);

        // Toml unchanged
        let toml_after = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before, toml_after);
    }

    #[tokio::test]
    async fn build_override_false() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init with auto_build (writes config with auto_build=true)
        crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true,
            None,
            false, // no auto_build for init
            false, // skip_gitignore
        )
        .await
        .unwrap();

        // Add a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        // --build false should skip build even if auto_build is true in toml
        let result = run(tmp.path(), &[], false, Some(false), false).await.unwrap();

        assert!(!result.auto_build);
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn reinfer_all_detects_glob_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // Two files with frontmatter + one bare file
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
        fs::write(blog_dir.join("bare.md"), "# No frontmatter\nJust content.")
            .unwrap();

        // Init with ignore_bare_files=true → title required=["**"]
        crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true,  // ignore bare files
            None,
            false, // no auto_build
            false,
        )
        .await
        .unwrap();

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_before = toml_before.fields.field.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title_before.required, vec!["**"]);

        // Flip include_bare_files to true
        let mut config = toml_before;
        config.scan.include_bare_files = true;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Reinfer all — globs should change even though types don't
        let result = run(tmp.path(), &[], true, Some(false), false).await.unwrap();
        assert!(result.has_changes());
        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "title");

        // Toml rewritten with narrower required
        let toml_after = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let title_after = toml_after.fields.field.iter().find(|f| f.name == "title").unwrap();
        assert!(!title_after.required.contains(&"**".to_string()));
    }

    #[tokio::test]
    async fn disappearing_field_stays_in_default_mode() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path()).await;

        // Remove tags from all files
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        // Default mode: tags should stay in toml even though it disappeared
        let result = run(tmp.path(), &[], false, Some(false), false).await.unwrap();

        assert!(!result.has_changes());

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "tags"));
    }
}
