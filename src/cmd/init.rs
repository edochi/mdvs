use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::output::{CommandOutput, DiscoveredField};
use crate::schema::config::MdvsToml;
use crate::schema::shared::{FieldTypeSerde, ScanConfig};
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
pub struct InitResult {
    pub path: PathBuf,
    pub files_scanned: usize,
    pub fields: Vec<DiscoveredField>,
    pub auto_build: bool,
    pub dry_run: bool,
}

impl CommandOutput for InitResult {
    fn format_human(&self) -> String {
        let mut out = String::new();

        out.push_str(&format!("{} files scanned\n", self.files_scanned));

        if self.fields.is_empty() {
            out.push_str("No frontmatter fields found.\n");
        } else {
            let name_width = self
                .fields
                .iter()
                .map(|f| f.name.len())
                .max()
                .unwrap()
                .max(5);
            let type_width = self
                .fields
                .iter()
                .map(|f| f.field_type.len())
                .max()
                .unwrap()
                .max(4);

            out.push('\n');
            out.push_str(&format!(
                " {:<name_width$}  {:<type_width$}  Count\n",
                "Field", "Type",
            ));
            out.push_str(&format!(" {}\n", "─".repeat(name_width + type_width + 10)));
            for field in &self.fields {
                out.push_str(&format!(
                    " {:<name_width$}  {:<type_width$}  {}/{}\n",
                    field.name, field.field_type, field.files_found, field.total_files,
                ));
            }
        }

        if self.dry_run {
            if self.auto_build {
                out.push_str("\nWould build index with model 'minishlab/potion-base-8M'\n");
            }
            out.push_str("(dry run, nothing written)\n");
        } else {
            out.push_str(&format!("\nInitialized mdvs in '{}'\n", self.path.display()));
        }

        out
    }
}

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

#[allow(clippy::too_many_arguments)]
pub async fn run(
    path: &Path,
    model: Option<&str>,
    revision: Option<&str>,
    glob: &str,
    force: bool,
    dry_run: bool,
    ignore_bare_files: bool,
    chunk_size: Option<usize>,
    auto_build: bool,
    skip_gitignore: bool,
) -> anyhow::Result<InitResult> {
    anyhow::ensure!(path.is_dir(), "'{}' is not a directory", path.display());

    let config_path = path.join("mdvs.toml");

    if !force && config_path.exists() {
        anyhow::bail!(
            "mdvs.toml already exists in '{}' (use --force to overwrite)",
            path.display()
        );
    }

    // Flag validation: build-related flags require --auto-build
    if !auto_build {
        if model.is_some() {
            anyhow::bail!("--model has no effect without --auto-build");
        }
        if revision.is_some() {
            anyhow::bail!("--revision has no effect without --auto-build");
        }
        if chunk_size.is_some() {
            anyhow::bail!("--chunk-size has no effect without --auto-build");
        }
    }

    let scan_config = ScanConfig {
        glob: glob.to_string(),
        include_bare_files: !ignore_bare_files,
        skip_gitignore,
    };
    let scanned = ScannedFiles::scan(path, &scan_config);

    anyhow::ensure!(
        !scanned.files.is_empty(),
        "no markdown files found in '{}'",
        path.display()
    );

    let schema = InferredSchema::infer(&scanned);
    let total_files = scanned.files.len();

    let result = InitResult {
        path: path.to_path_buf(),
        files_scanned: total_files,
        fields: schema
            .fields
            .iter()
            .map(|f| DiscoveredField {
                name: f.name.clone(),
                field_type: FieldTypeSerde::from(&f.field_type).to_string(),
                files_found: f.files.len(),
                total_files,
            })
            .collect(),
        auto_build,
        dry_run,
    };

    if dry_run {
        return Ok(result);
    }

    // Apply defaults for build-related flags
    let model_name = model.unwrap_or(DEFAULT_MODEL);
    let max_chunk_size = chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE);

    let toml_doc = MdvsToml::from_inferred(
        &schema,
        scan_config,
        model_name,
        revision,
        max_chunk_size,
        auto_build,
    );
    toml_doc.write(&config_path)?;

    if auto_build {
        crate::cmd::build::run(path, None, None, None, false).await?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
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

        fs::write(
            dir.join("bare.md"),
            "# No frontmatter\nJust content.",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true, // dry_run
            true, // ignore_bare_files
            None,
            true,  // auto_build
            false, // skip_gitignore
        )
        .await;

        let result = result.unwrap();
        assert!(result.dry_run);
        assert!(!tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[tokio::test]
    async fn dry_run_result_fields() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true, // dry_run
            true, // ignore_bare_files
            None,
            false, // no auto_build
            false, // skip_gitignore
        )
        .await
        .unwrap();

        assert_eq!(result.files_scanned, 2); // bare.md excluded
        assert!(!result.fields.is_empty());
        assert!(result.dry_run);
        assert!(!result.auto_build);

        // Check field structure
        let title = result.fields.iter().find(|f| f.name == "title").unwrap();
        assert_eq!(title.field_type, "String");
        assert_eq!(title.files_found, 2);
        assert_eq!(title.total_files, 2);
    }

    #[tokio::test]
    async fn existing_config_no_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false, // no force
            true,
            true,
            None,
            true,
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"));
        assert!(err.contains("--force"));
    }

    #[tokio::test]
    async fn existing_config_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        // force + dry_run: bypasses the existing-file check, skips build
        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            true, // force
            true, // dry_run
            true,
            None,
            true,
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        // empty directory, no .md files

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true,
            true,
            None,
            true,
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no markdown files"));
    }

    #[tokio::test]
    async fn flag_validation_model_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            Some("some-model"),
            None,
            "**",
            false,
            true,
            true,
            None,
            false, // no auto_build
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--model has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn flag_validation_revision_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            None,
            Some("abc123"),
            "**",
            false,
            true,
            true,
            None,
            false,
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--revision has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn flag_validation_chunk_size_without_auto_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            true,
            true,
            Some(512),
            false,
            false, // skip_gitignore
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--chunk-size has no effect without --auto-build"));
    }

    #[tokio::test]
    async fn no_auto_build_skips_build() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false, // not dry_run — writes config
            true,
            None,
            false, // no auto_build
            false, // skip_gitignore
        )
        .await
        .unwrap();

        // Config written, but no .mdvs/ directory
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join(".mdvs").exists());
        assert!(!result.auto_build);

        // Verify config has no build sections
        let toml_doc = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(toml_doc.embedding_model.is_none());
        assert!(toml_doc.chunking.is_none());
        assert!(toml_doc.search.is_none());
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let model = "minishlab/potion-base-8M";
        let result = run(
            tmp.path(),
            Some(model),
            None,
            "**",
            false,
            false, // not dry_run — full pipeline
            true,  // ignore bare files
            None,
            true,  // auto_build
            false, // skip_gitignore
        )
        .await;

        let result = result.unwrap();
        assert!(result.auto_build);
        assert!(!result.dry_run);

        // Verify files exist (build creates .mdvs/)
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(tmp.path().join(".mdvs").is_dir());

        // Verify config contents
        let toml_doc = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_doc.scan.glob, "**");
        assert!(!toml_doc.scan.include_bare_files);
        assert_eq!(toml_doc.embedding_model.as_ref().unwrap().name, model);
        assert!(!toml_doc.fields.field.is_empty());
    }
}
