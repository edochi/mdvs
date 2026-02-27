use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::index::embed::{resolve_revision, Embedder, ModelConfig};
use crate::schema::config::MdvsToml;
use crate::schema::lock::MdvsLock;
use crate::schema::shared::FieldTypeSerde;
use std::path::Path;

pub fn run(
    path: &Path,
    model_name: &str,
    revision: Option<&str>,
    glob: &str,
    force: bool,
    dry_run: bool,
    ignore_bare_files: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(path.is_dir(), "'{}' is not a directory", path.display());

    let config_path = path.join("mdvs.toml");
    let lock_path = path.join("mdvs.lock");

    if !force && config_path.exists() {
        anyhow::bail!(
            "mdvs.toml already exists in '{}' (use --force to overwrite)",
            path.display()
        );
    }

    let include_bare_files = !ignore_bare_files;
    let scanned = ScannedFiles::scan(path, glob, include_bare_files);

    anyhow::ensure!(
        !scanned.files.is_empty(),
        "no markdown files found in '{}'",
        path.display()
    );

    let schema = InferredSchema::infer(&scanned);

    print_discovery_table(&scanned, &schema);

    if dry_run {
        return Ok(());
    }

    // Download model and resolve identity
    eprintln!("Loading model {model_name}...");
    let config = ModelConfig::Model2Vec {
        model_id: model_name.to_string(),
        revision: revision.map(|s| s.to_string()),
    };
    let _embedder = Embedder::load(&config);
    let model_revision = resolve_revision(model_name)
        .unwrap_or_else(|| "unknown".to_string());

    let toml_doc = MdvsToml::from_inferred(
        &schema,
        glob,
        include_bare_files,
        model_name,
        revision,
    );
    toml_doc.write(&config_path)?;

    let lock_doc = MdvsLock::from_inferred(
        &schema,
        &scanned,
        glob,
        include_bare_files,
        model_name,
        &model_revision,
    );
    lock_doc.write(&lock_path)?;

    let mdvs_dir = path.join(".mdvs");
    std::fs::create_dir_all(&mdvs_dir)?;

    eprintln!("Initialized mdvs in '{}'", path.display());
    Ok(())
}

fn print_discovery_table(scanned: &ScannedFiles, schema: &InferredSchema) {
    let total = scanned.files.len();
    eprintln!("{total} markdown files scanned");

    if schema.fields.is_empty() {
        eprintln!("No frontmatter fields found.");
        return;
    }

    // Compute column widths
    let name_width = schema
        .fields
        .iter()
        .map(|f| f.name.len())
        .max()
        .unwrap()
        .max(5);
    let type_width = schema
        .fields
        .iter()
        .map(|f| FieldTypeSerde::from(&f.field_type).to_string().len())
        .max()
        .unwrap()
        .max(4);

    eprintln!();
    eprintln!(
        " {:<name_width$}  {:<type_width$}  Count",
        "Field", "Type",
    );
    eprintln!(
        " {}",
        "─".repeat(name_width + type_width + 10),
    );
    for field in &schema.fields {
        let type_str = FieldTypeSerde::from(&field.field_type).to_string();
        let count = field.files.len();
        eprintln!(
            " {:<name_width$}  {:<type_width$}  {count}/{total}",
            field.name, type_str,
        );
    }
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

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let result = run(
            tmp.path(),
            "test-model",
            None,
            "**",
            false,
            true, // dry_run
            true, // ignore_bare_files
        );

        assert!(result.is_ok());
        assert!(!tmp.path().join("mdvs.toml").exists());
        assert!(!tmp.path().join("mdvs.lock").exists());
        assert!(!tmp.path().join(".mdvs").exists());
    }

    #[test]
    fn existing_config_no_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        let result = run(
            tmp.path(),
            "test-model",
            None,
            "**",
            false, // no force
            true,
            true,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("already exists"));
        assert!(err.contains("--force"));
    }

    #[test]
    fn existing_config_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        fs::write(tmp.path().join("mdvs.toml"), "existing").unwrap();

        // force + dry_run: bypasses the existing-file check, skips model download
        let result = run(
            tmp.path(),
            "test-model",
            None,
            "**",
            true, // force
            true, // dry_run
            true,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn no_markdown_files() {
        let tmp = tempfile::tempdir().unwrap();
        // empty directory, no .md files

        let result = run(
            tmp.path(),
            "test-model",
            None,
            "**",
            false,
            true,
            true,
        );

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("no markdown files"));
    }

    #[test]
    fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let model = "minishlab/potion-base-8M";
        let result = run(
            tmp.path(),
            model,
            None,
            "**",
            false,
            false, // not dry_run — full pipeline
            true,  // ignore bare files
        );

        assert!(result.is_ok(), "init failed: {:?}", result);

        // Verify files exist
        assert!(tmp.path().join("mdvs.toml").exists());
        assert!(tmp.path().join("mdvs.lock").exists());
        assert!(tmp.path().join(".mdvs").is_dir());

        // Verify config contents
        let toml_doc = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(toml_doc.config.glob, "**");
        assert!(!toml_doc.config.include_bare_files);
        assert_eq!(toml_doc.model.name, model);
        assert!(!toml_doc.fields.is_empty());

        // Verify lock contents
        let lock_doc = MdvsLock::read(&tmp.path().join("mdvs.lock")).unwrap();
        assert_eq!(lock_doc.model.name, model);
        assert!(lock_doc.model.revision.is_some());
        assert_eq!(lock_doc.files.len(), 2); // 2 files with frontmatter (bare excluded)
        assert!(!lock_doc.fields.is_empty());
    }
}
