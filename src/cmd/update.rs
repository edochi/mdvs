use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::output::{ChangedField, CommandOutput, DiscoveredField};
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::shared::FieldTypeSerde;
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct UpdateResult {
    pub files_scanned: usize,
    pub added: Vec<DiscoveredField>,
    pub changed: Vec<ChangedField>,
    pub removed: Vec<String>,
    pub unchanged: usize,
    pub auto_build: bool,
    pub dry_run: bool,
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

        out
    }
}

pub fn run(
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

    // Build old_types map from targets for comparison
    let old_types: HashMap<&str, &FieldTypeSerde> = targets
        .iter()
        .map(|f| (f.name.as_str(), &f.field_type))
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

        if let Some(old_type) = old_types.get(inf.name.as_str()) {
            // Was a target for reinference
            if **old_type == new_type {
                unchanged += 1;
            } else {
                changed.push(ChangedField {
                    name: inf.name.clone(),
                    old_type: old_type.to_string(),
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
    let mut removed: Vec<String> = old_types
        .keys()
        .filter(|name| !schema.fields.iter().any(|f| f.name == **name))
        .map(|name| name.to_string())
        .collect();
    removed.sort();

    let should_build = build_flag.unwrap_or(config.update.auto_build);

    let result = UpdateResult {
        files_scanned: total_files,
        added,
        changed,
        removed,
        unchanged,
        auto_build: should_build && !dry_run,
        dry_run,
    };

    if dry_run || !result.has_changes() {
        return Ok(result);
    }

    // Update fields and write
    config.fields.field = new_fields;
    config.write(&config_path)?;

    if should_build {
        crate::cmd::build::run(path, None, None, None, false)?;
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

    fn init_no_build(dir: &Path) {
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
        .unwrap();
    }

    #[test]
    fn no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let result = run(tmp.path(), &[], false, Some(false), false).unwrap();

        assert!(!result.has_changes());
        assert_eq!(result.files_scanned, 2);
        assert_eq!(result.unchanged, 3); // title, tags, draft
    }

    #[test]
    fn new_fields_discovered() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        // Add a file with a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let result = run(tmp.path(), &[], false, Some(false), false).unwrap();

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

    #[test]
    fn reinfer_changes_type() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

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
        .unwrap();

        assert_eq!(result.changed.len(), 1);
        assert_eq!(result.changed[0].name, "tags");
        assert_eq!(result.changed[0].new_type, "String");
        assert!(result.added.is_empty());
        assert!(result.removed.is_empty());
    }

    #[test]
    fn reinfer_removes_disappeared() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

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
        .unwrap();

        assert_eq!(result.removed, vec!["tags"]);
        assert!(result.changed.is_empty());
        assert!(result.added.is_empty());

        // Verify toml no longer has tags
        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(!toml.fields.field.iter().any(|f| f.name == "tags"));
    }

    #[test]
    fn reinfer_unknown_field_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let result = run(
            tmp.path(),
            &["nonexistent".to_string()],
            false,
            Some(false),
            false,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("field 'nonexistent' is not in mdvs.toml"));
    }

    #[test]
    fn reinfer_and_reinfer_all_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let result = run(
            tmp.path(),
            &["tags".to_string()],
            true, // reinfer_all
            Some(false),
            false,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("cannot use --reinfer and --reinfer-all together"));
    }

    #[test]
    fn reinfer_all_preserves_config() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        let toml_before = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), &[], true, Some(false), false).unwrap();

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

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        let toml_before = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), &[], false, Some(false), true).unwrap();

        assert!(result.dry_run);
        assert_eq!(result.added.len(), 1);

        // Toml unchanged
        let toml_after = fs::read_to_string(tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_before, toml_after);
    }

    #[test]
    fn build_override_false() {
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
        .unwrap();

        // Add a new field
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\nauthor: Alice\n---\n# New\nContent.",
        )
        .unwrap();

        // --build false should skip build even if auto_build is true in toml
        let result = run(tmp.path(), &[], false, Some(false), false).unwrap();

        assert!(!result.auto_build);
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn disappearing_field_stays_in_default_mode() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_no_build(tmp.path());

        // Remove tags from all files
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        // Default mode: tags should stay in toml even though it disappeared
        let result = run(tmp.path(), &[], false, Some(false), false).unwrap();

        assert!(!result.has_changes());

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml.fields.field.iter().any(|f| f.name == "tags"));
    }
}
