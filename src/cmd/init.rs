use crate::cmd::build::BuildResult;
use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::index::storage::check_reserved_names;
use crate::output::{format_file_count, CommandOutput, DiscoveredField};
use crate::schema::config::MdvsToml;
use crate::schema::shared::{FieldTypeSerde, ScanConfig};
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{info, instrument};

/// Result of the `init` command: discovered fields and optional build output.
#[derive(Debug, Serialize)]
pub struct InitResult {
    /// Directory where `mdvs.toml` was written.
    pub path: PathBuf,
    /// Number of markdown files scanned.
    pub files_scanned: usize,
    /// Fields inferred from frontmatter.
    pub fields: Vec<DiscoveredField>,
    /// Whether a build was triggered after initialization.
    pub auto_build: bool,
    /// Whether this was a dry run (no files written).
    pub dry_run: bool,
    /// Build result, if a build was triggered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build_result: Option<BuildResult>,
    /// Scan glob pattern (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    /// Wall-clock time for the init operation in milliseconds (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

impl CommandOutput for InitResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        let field_summary = if self.fields.is_empty() {
            "no fields found".to_string()
        } else {
            format!("{} field(s)", self.fields.len())
        };
        let dry_run_suffix = if self.dry_run { " (dry run)" } else { "" };
        out.push_str(&format!(
            "Initialized {} — {field_summary}{dry_run_suffix}\n",
            format_file_count(self.files_scanned)
        ));

        if !self.fields.is_empty() {
            out.push('\n');
            if verbose {
                // Record tables per field
                for field in &self.fields {
                    let mut builder = Builder::default();
                    builder.push_record([
                        format!("\"{}\"", field.name),
                        field.field_type.clone(),
                        format!("{}/{}", field.files_found, field.total_files),
                    ]);
                    let mut detail_lines = Vec::new();
                    if let Some(ref req) = field.required {
                        if !req.is_empty() {
                            detail_lines.push("  required:".to_string());
                            for g in req {
                                detail_lines.push(format!("    - \"{g}\""));
                            }
                        }
                    }
                    if let Some(ref allowed) = field.allowed {
                        detail_lines.push("  allowed:".to_string());
                        for g in allowed {
                            detail_lines.push(format!("    - \"{g}\""));
                        }
                    }
                    builder.push_record([detail_lines.join("\n"), String::new(), String::new()]);
                    let mut table = builder.build();
                    style_record(&mut table, 3);
                    out.push_str(&format!("{table}\n"));
                }
            } else {
                // Compact table
                let mut builder = Builder::default();
                for field in &self.fields {
                    builder.push_record([
                        format!("\"{}\"", field.name),
                        field.field_type.clone(),
                        format!("{}/{}", field.files_found, field.total_files),
                    ]);
                }
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
        }

        if self.dry_run {
            if self.auto_build {
                out.push_str("\nWould build index with model 'minishlab/potion-base-8M'\n");
            }
            out.push_str("(dry run, nothing written)\n");
        } else {
            out.push_str(&format!(
                "\nInitialized mdvs in '{}'\n",
                self.path.display()
            ));
        }

        // Verbose footer
        if verbose {
            if let (Some(glob), Some(ms)) = (&self.glob, self.elapsed_ms) {
                out.push_str(&format!(
                    "\n{} | glob: \"{glob}\" | {ms}ms\n",
                    format_file_count(self.files_scanned)
                ));
            }
        }

        if let Some(ref br) = self.build_result {
            out.push('\n');
            out.push_str(&br.format_text(verbose));
        }

        out
    }
}

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

/// Scan a directory, infer frontmatter schema, write `mdvs.toml`, and optionally build.
#[allow(clippy::too_many_arguments)]
#[instrument(name = "init", skip_all)]
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
    verbose: bool,
) -> anyhow::Result<InitResult> {
    let start = Instant::now();
    info!(path = %path.display(), "initializing");

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

    info!(fields = schema.fields.len(), "schema inferred");

    let mut result = InitResult {
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
                allowed: if verbose {
                    Some(f.allowed.clone())
                } else {
                    None
                },
                required: if verbose {
                    Some(f.required.clone())
                } else {
                    None
                },
            })
            .collect(),
        auto_build,
        dry_run,
        build_result: None,
        glob: if verbose {
            Some(glob.to_string())
        } else {
            None
        },
        elapsed_ms: None,
    };

    if dry_run {
        if verbose {
            result.elapsed_ms = Some(start.elapsed().as_millis() as u64);
        }
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

    // Validate field names don't collide with internal column names
    let field_names: Vec<String> = schema.fields.iter().map(|f| f.name.clone()).collect();
    check_reserved_names(&field_names, toml_doc.internal_prefix())?;

    toml_doc.write(&config_path)?;

    if auto_build {
        result.build_result = Some(crate::cmd::build::run(path, None, None, None, false).await?);
    }

    if verbose {
        result.elapsed_ms = Some(start.elapsed().as_millis() as u64);
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

        fs::write(dir.join("bare.md"), "# No frontmatter\nJust content.").unwrap();
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
            false, // verbose
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
