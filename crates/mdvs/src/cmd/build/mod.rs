mod classify;
mod config_mutate;
mod embed;
mod write;

use std::{path::Path, time::Instant};

use classify::{ClassifyData, classify_step};
#[cfg(test)]
use config_mutate::DEFAULT_CHUNK_SIZE;
use config_mutate::detect_config_changes;
pub(crate) use config_mutate::mutate_config;
use embed::{embed_step, load_embedder_step};
use tracing::instrument;
use write::{WritePlan, file_rows, write_index_step};

// Imported for tests' `use super::*;` — tests construct full MdvsToml
// fixtures and need this section type, which production code here does
// not use.
#[cfg(test)]
use crate::schema::config::SearchConfig;
use crate::{
    cmd::steps::{auto_update_step, read_config_step, scan_step},
    discover::{field_type::FieldType, scan::ScannedFiles},
    index::{
        backend::Backend,
        embed::Embedder,
        storage::{BuildMetadata, compute_schema_hash},
    },
    outcome::{Outcome, ValidateOutcome, commands::BuildOutcome},
    output::{BuildFileDetail, NewField},
    schema::{
        config::MdvsToml,
        shared::{ChunkingConfig, EmbeddingModelConfig},
    },
    step::{CommandResult, ErrorKind, StepEntry, elapsed_ms},
};

// ============================================================================
// run()
// ============================================================================

/// Validate frontmatter, chunk, embed, and write the Lance dataset to `.mdvs/`.
#[instrument(name = "build", skip_all)]
pub async fn run(
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
    no_update: bool,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // 1. Read config
    let Ok((mut config, config_path)) = read_config_step(path, &mut steps) else {
        return CommandResult::failed_from_steps(steps, start);
    };

    let should_update = !no_update && config.build.as_ref().is_some_and(|b| b.auto_update);

    // 2. Mutate config (inline, not a step)
    let mutation_error = mutate_config(
        &mut config,
        path,
        set_model,
        set_revision,
        set_chunk_size,
        force,
    );

    if let Some(msg) = mutation_error {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return CommandResult::failed_from_steps(steps, start);
    }

    // 3. Core build pipeline (scan → auto-update → validate → classify → embed → write)
    let Ok((build_outcome, _embedder)) = build_core(
        path,
        &mut config,
        &config_path,
        force,
        should_update,
        &mut steps,
    )
    .await
    else {
        return CommandResult::failed_from_steps(steps, start);
    };

    CommandResult {
        steps,
        result: Ok(Outcome::Build(Box::new(build_outcome))),
        elapsed_ms: elapsed_ms(start),
    }
}

// ============================================================================
// build_core() — shared pipeline, called by build::run() and search::run()
// ============================================================================

/// Core build pipeline: scan → auto-update → validate → classify → embed → write index.
///
/// The final write step dispatches across three paths (in `cmd::build::write`):
/// **skip** (nothing changed and not a full rebuild), **full overwrite** (first
/// build or `--force`), or **incremental** (delete the rows for new/edited/removed
/// files, append the freshly embedded chunks, refresh metadata, optimize).
///
/// Returns `BuildOutcome` + optional `Embedder` (for reuse by search) on success.
/// On failure, pushes error steps and returns `Err(())` — the caller constructs
/// the failed `CommandResult` from the steps.
///
/// Public for profiling and benchmarking (the per-phase timings in `steps` are
/// the closest thing to a profile of a real `mdvs build` invocation).
pub async fn build_core(
    path: &Path,
    config: &mut MdvsToml,
    config_path: &Path,
    force: bool,
    auto_update: bool,
    steps: &mut Vec<StepEntry>,
) -> Result<(BuildOutcome, Option<Embedder>), ()> {
    // 1. Scan
    let scanned = scan_step(path, &config.scan, steps)?;

    // 2. Auto-update: infer new fields, write config if changed
    if auto_update {
        auto_update_step(config, config_path, &scanned, steps)?;
    }

    // 3. Validate
    let new_fields = validate_step(path, &scanned, config, steps)?;

    // 4. Pre-checks for classify
    let PreChecked {
        schema_fields,
        embedding,
        chunking,
        backend,
    } = pre_check_step(path, config, force, steps).await?;

    // 5. Classify
    let full_rebuild = force || !backend.exists();
    let classify_data = classify_step(&backend, &scanned, full_rebuild, steps).await?;

    // 6. Load model and check its dimension against the index
    let embedder = load_embedder_step(embedding, &backend, &classify_data, steps).await?;

    // 7. Embed files
    let built_at = chrono::Utc::now().timestamp_micros();
    let (new_chunk_rows, embedded_details) = if let Some(emb) = &embedder {
        let embedded = embed_step(
            &classify_data.needs_embedding,
            chunking.max_chunk_size,
            emb,
            steps,
        )
        .await?;
        (embedded.chunk_rows, embedded.details)
    } else {
        steps.push(StepEntry::skipped());
        (Vec::new(), Vec::new())
    };

    // 8. Write index
    let file_rows = file_rows(&scanned, &classify_data, built_at).map_err(|e| {
        steps.push(StepEntry::err(ErrorKind::Application, format!("{e:#}"), 0));
    })?;
    let build_meta = BuildMetadata {
        embedding_model: embedding.clone(),
        chunking: chunking.clone(),
        glob: config.scan.glob.clone(),
        built_at: chrono::Utc::now().to_rfc3339(),
        schema_hash: compute_schema_hash(config),
    };
    let plan = WritePlan::decide(&classify_data, &file_rows, &new_chunk_rows);
    write_index_step(&backend, &schema_fields, plan, build_meta, steps).await?;

    let outcome = build_outcome(
        classify_data,
        file_rows.len(),
        new_chunk_rows.len(),
        embedded_details,
        new_fields,
    );
    Ok((outcome, embedder))
}

/// Validate the scanned frontmatter against `config`.
///
/// Fails when the scan found no markdown files, when validation itself
/// errors, or when any violation is found. Returns the fields seen in
/// frontmatter but absent from the config.
fn validate_step(
    path: &Path,
    scanned: &ScannedFiles,
    config: &MdvsToml,
    steps: &mut Vec<StepEntry>,
) -> Result<Vec<NewField>, ()> {
    if scanned.files.is_empty() {
        steps.push(StepEntry::err(
            ErrorKind::User,
            format!("no markdown files found in '{}'", path.display()),
            0,
        ));
        return Err(());
    }

    let validate_start = Instant::now();
    let check_result = crate::cmd::check::validate(scanned, config, false).map_err(|e| {
        steps.push(StepEntry::err(
            ErrorKind::Application,
            e.to_string(),
            elapsed_ms(validate_start),
        ));
    })?;
    steps.push(StepEntry::ok(
        Outcome::Validate(ValidateOutcome {
            files_checked: check_result.files_checked,
            violations: check_result.field_violations.clone(),
            new_fields: check_result.new_fields.clone(),
        }),
        elapsed_ms(validate_start),
    ));

    let violations = check_result.field_violations;
    if !violations.is_empty() {
        steps.push(StepEntry::err(
            ErrorKind::User,
            format!(
                "{} violation(s) found. Run `mdvs check` for details.",
                violations.len()
            ),
            0,
        ));
        return Err(());
    }
    Ok(check_result.new_fields)
}

/// What the build needs from the config once it has been checked.
struct PreChecked<'a> {
    /// Every configured field with its parsed type.
    schema_fields: Vec<(String, FieldType)>,
    /// The `[embedding_model]` section.
    embedding: &'a EmbeddingModelConfig,
    /// The `[chunking]` section.
    chunking: &'a ChunkingConfig,
    /// The index backend for `path`.
    backend: Backend,
}

/// Parse the field types, require the `[embedding_model]` and `[chunking]`
/// sections, and refuse a config that changed since the last build unless
/// `force` is set. Each failure is an untimed error step.
async fn pre_check_step<'a>(
    path: &Path,
    config: &'a MdvsToml,
    force: bool,
    steps: &mut Vec<StepEntry>,
) -> Result<PreChecked<'a>, ()> {
    let schema_fields = config
        .fields
        .field
        .iter()
        .map(|f| {
            let ft = FieldType::try_from(&f.field_type)
                .map_err(|e| format!("invalid field type for '{}': {}", f.name, e))?;
            Ok((f.name.clone(), ft))
        })
        .collect::<Result<Vec<_>, String>>()
        .map_err(|msg| steps.push(StepEntry::err(ErrorKind::Application, msg, 0)))?;

    let Some(embedding) = config.embedding_model.as_ref() else {
        steps.push(StepEntry::err(
            ErrorKind::User,
            "missing [embedding_model] in mdvs.toml".into(),
            0,
        ));
        return Err(());
    };
    let Some(chunking) = config.chunking.as_ref() else {
        steps.push(StepEntry::err(
            ErrorKind::User,
            "missing [chunking] in mdvs.toml".into(),
            0,
        ));
        return Err(());
    };

    let backend = Backend::lance(path);
    if let Some(msg) = detect_config_changes(&backend, embedding, chunking, config, force).await {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return Err(());
    }
    Ok(PreChecked {
        schema_fields,
        embedding,
        chunking,
        backend,
    })
}

/// Summarize a completed build.
fn build_outcome(
    classify_data: ClassifyData<'_>,
    files_total: usize,
    chunks_embedded: usize,
    embedded_files: Vec<BuildFileDetail>,
    new_fields: Vec<NewField>,
) -> BuildOutcome {
    let chunks_unchanged = classify_data.retained_chunks.len();
    let files_embedded = classify_data.needs_embedding.len();
    BuildOutcome {
        full_rebuild: classify_data.full_rebuild,
        files_total,
        files_embedded,
        files_unchanged: files_total - files_embedded,
        files_removed: classify_data.removed_count,
        chunks_total: chunks_unchanged + chunks_embedded,
        chunks_embedded,
        chunks_unchanged,
        chunks_removed: classify_data.chunks_removed,
        new_fields,
        embedded_files,
        removed_files: classify_data.removed_details,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        fs,
    };

    use super::*;
    use crate::{
        cmd::init::{InitOptions, InitScanFlags},
        schema::config::MdvsToml,
        step::StepError,
    };

    fn unwrap_build(result: &CommandResult) -> &BuildOutcome {
        match &result.result {
            Ok(Outcome::Build(o)) => o,
            other => panic!("expected Ok(Build), got: {other:?}"),
        }
    }

    fn unwrap_error(result: &CommandResult) -> &StepError {
        match &result.result {
            Err(e) => e,
            other => panic!("expected Err, got: {other:?}"),
        }
    }

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text about the world.",
        )
        .unwrap();
    }

    /// A second build over an unchanged vault must short-circuit the
    /// `write_index` step (Skipped, not Completed). The first build is a full
    /// rebuild and must Complete it.
    #[tokio::test]
    async fn second_build_skips_write_index_when_nothing_changed() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // init
        let init_out = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags { ignore_bare_files: true, skip_gitignore: // include bare files
            false },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_out));

        // first build — full rebuild, must WRITE the index
        let first = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&first),
            "first build failed: {first:#?}"
        );
        let wrote_first = first.steps.iter().any(|s| {
            matches!(
                s,
                StepEntry::Completed(c) if matches!(c.outcome, Outcome::WriteIndex(_))
            )
        });
        assert!(
            wrote_first,
            "first build should have a Completed WriteIndex step"
        );

        // second build — nothing changed, must SKIP the index write
        let second = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&second),
            "second build failed: {second:#?}"
        );
        let skipped_write = matches!(second.steps.last(), Some(StepEntry::Skipped));
        let no_completed_write = !second.steps.iter().any(|s| {
            matches!(
                s,
                StepEntry::Completed(c) if matches!(c.outcome, Outcome::WriteIndex(_))
            )
        });
        assert!(
            skipped_write && no_completed_write,
            "second build should skip WriteIndex: {second:#?}"
        );

        // The Lance dataset still exists and is queryable.
        assert!(tmp.path().join(".mdvs/index.lance").exists());
        let backend = Backend::lance(tmp.path());
        let file_index = backend.read_file_index().await.unwrap();
        assert_eq!(file_index.len(), 2);
    }

    /// When a new file appears between two builds, the incremental write
    /// path must persist it: the new file's chunks must be visible in the
    /// index, unchanged files retain their chunks, and the `WriteIndex` step
    /// is Completed (not Skipped).
    #[tokio::test]
    async fn third_build_persists_new_file_via_incremental_path() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_out = crate::cmd::init::run(
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
        assert!(!crate::step::has_failed(&init_out));

        // First build: full rebuild over 2 files.
        let first = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&first));
        let backend = Backend::lance(tmp.path());
        let chunks_after_first = backend.read_chunk_rows().await.unwrap().len();
        assert!(chunks_after_first >= 2);

        // Drop a third file into the vault. Match the schema inferred
        // from post1/post2 (title + draft + tags) so validation passes.
        let blog = tmp.path().join("blog");
        fs::write(
            blog.join("post3.md"),
            "---\ntitle: Third\ndraft: false\ntags:\n  - new\n---\n# Third\nFresh body content that should produce a chunk.",
        )
        .unwrap();

        // Second build: incremental path. WriteIndex must Complete (not
        // Skipped), the new file's chunks must be persisted, and the
        // two unchanged files' chunks must still be present.
        let second = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&second),
            "incremental build failed: {second:#?}"
        );
        let wrote = second.steps.iter().any(|s| {
            matches!(
                s,
                StepEntry::Completed(c) if matches!(c.outcome, Outcome::WriteIndex(_))
            )
        });
        assert!(wrote, "incremental build should Complete WriteIndex");

        let file_index = backend.read_file_index().await.unwrap();
        assert_eq!(file_index.len(), 3, "all three files should be indexed");
        let chunks_after_second = backend.read_chunk_rows().await.unwrap();
        assert!(
            chunks_after_second.len() > chunks_after_first,
            "second build should add chunks for the new file (was {chunks_after_first}, now {})",
            chunks_after_second.len()
        );

        // Verify the third file's chunks are present by file_id.
        let third_id = file_index
            .iter()
            .find(|f| f.filename.ends_with("post3.md"))
            .expect("post3.md should be in the file index")
            .file_id
            .clone();
        let third_chunks: Vec<_> = chunks_after_second
            .iter()
            .filter(|c| c.file_id == third_id)
            .collect();
        assert!(
            !third_chunks.is_empty(),
            "post3.md should have at least one chunk"
        );
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only, no build)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags { ignore_bare_files: true, skip_gitignore: // ignore bare files
            false },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "first build failed: {output:#?}"
        );

        // Run build again (tests standalone rebuild)
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "build failed: {output:#?}"
        );
        assert!(!crate::step::has_failed(&output));

        // Verify the Lance index exists
        assert!(tmp.path().join(".mdvs/index.lance").exists());

        let backend = Backend::lance(tmp.path());

        // Verify file count (2 files with frontmatter)
        let file_index = backend.read_file_index().await.unwrap();
        assert_eq!(file_index.len(), 2);

        // Verify chunks exist with a positive embedding dimension
        let chunk_rows = backend.read_chunk_rows().await.unwrap();
        assert!(!chunk_rows.is_empty());
        assert!(backend.embedding_dimension().await.unwrap().unwrap() > 0);

        // Verify build metadata round-trips
        let meta = backend.read_metadata().await.unwrap();
        assert!(meta.is_some(), "build metadata should be present");
        let meta = meta.unwrap();
        assert_eq!(meta.embedding_model.name, "mock");
        assert_eq!(meta.chunking.max_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(meta.glob, "**");
    }

    #[tokio::test]
    async fn dimension_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        // Overwrite the Lance index with a wrong-dimension (2) embedding,
        // reusing the existing metadata so only the dimension differs.
        overwrite_index_with_bad_dimension(tmp.path()).await;

        // Build should fail with dimension mismatch when the model loads
        // (all real files now read as "new" against the bad index).
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("dimension mismatch"));
    }

    /// Test helper: replace the Lance index with a single chunk whose
    /// embedding has dimension 2, preserving the existing build metadata.
    async fn overwrite_index_with_bad_dimension(root: &std::path::Path) {
        use crate::{
            discover::field_type::FieldType,
            index::storage::{ChunkRow, FileRow},
        };
        let backend = Backend::lance(root);
        let meta = backend.read_metadata().await.unwrap().unwrap();
        let schema_fields = vec![("title".to_string(), FieldType::String)];
        let bad_files = vec![FileRow {
            file_id: "bad".into(),
            filename: "bad.md".into(),
            frontmatter: None,
            content_hash: "h".into(),
            built_at: 0,
        }];
        let bad_chunks = vec![ChunkRow {
            chunk_id: "bad".into(),
            file_id: "bad".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            chunk_text: String::new(),
            embedding: vec![0.1, 0.2], // dim=2
        }];
        backend
            .write_index(&schema_fields, &bad_files, &bad_chunks, meta)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn dimension_mismatch_with_force_succeeds() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        // Overwrite the Lance index with a wrong-dimension (2) embedding.
        overwrite_index_with_bad_dimension(tmp.path()).await;

        // Build with --force should succeed despite dimension mismatch
        let output = run(tmp.path(), None, None, None, true, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "expected success with --force, got failed step"
        );
        assert!(!crate::step::has_failed(&output));
        let result = unwrap_build(&output);
        assert!(result.full_rebuild);
    }

    #[tokio::test]
    async fn missing_build_sections_filled() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init (schema only, no build sections in toml)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&output));

        // Verify no model/chunking sections (auto-flag sections are present from init)
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());

        // Build should fill defaults and succeed
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "build failed: {output:#?}"
        );

        // Verify sections were written. Under the `testing-mocks` feature the
        // default is the mock embedder; otherwise it's `DEFAULT_MODEL`.
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        #[cfg(any(test, feature = "testing-mocks"))]
        let expected_name = "mock";
        #[cfg(not(any(test, feature = "testing-mocks")))]
        let expected_name = DEFAULT_MODEL;
        assert_eq!(config.embedding_model.as_ref().unwrap().name, expected_name);
        assert!(config.embedding_model.as_ref().unwrap().revision.is_none());
        assert_eq!(
            config.chunking.as_ref().unwrap().max_chunk_size,
            DEFAULT_CHUNK_SIZE
        );
        assert_eq!(config.search.as_ref().unwrap().default_limit, 10);

        // Verify index was created
        assert!(tmp.path().join(".mdvs/index.lance").exists());
    }

    #[tokio::test]
    async fn set_model_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init (schema only)
        let output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        // Try to change model without --force
        let output = run(tmp.path(), Some("other-model"), None, None, false, true).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("--force"));
    }

    #[tokio::test]
    async fn set_chunk_size_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let output = run(tmp.path(), None, None, Some(512), false, true).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("--force"));
    }

    #[tokio::test]
    async fn set_model_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections)
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        // Change chunk size with --force (same model so no dimension mismatch)
        let output = run(tmp.path(), None, None, Some(512), true, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "build with --force failed: {output:#?}"
        );

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.chunking.as_ref().unwrap().max_chunk_size, 512);
    }

    #[tokio::test]
    async fn manual_config_change_detected() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index (creates build sections + Lance dataset)
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("config changed since last build"));
        assert!(err.message.contains("chunk_size"));

        // Build with --force should succeed
        let output = run(tmp.path(), None, None, None, true, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "build with --force failed: {output:#?}"
        );
    }

    #[tokio::test]
    async fn build_aborts_on_wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // draft is string "yes" in the file but declared as Boolean in toml
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Hello\nBody.",
        )
        .unwrap();

        let mut config = MdvsToml {
            default_output_format: None,
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: crate::schema::shared::FrontmatterFormat::Auto,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        // Declare as Boolean, but file has String → WrongType violation
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                ],
                max_categories: None,
                min_category_repetition: None,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "mock".into(),
                name: "mock".into(),
                revision: None,
                dim: Some(256),
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            crate::step::has_violations(&output),
            "expected validation violations"
        );
    }

    #[tokio::test]
    async fn build_aborts_on_missing_required() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // post1 has tags, post2 does not — tags required in blog/**
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n---\n# Hello\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\n---\n# World\nBody.",
        )
        .unwrap();

        let mut config = MdvsToml {
            default_output_format: None,
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: crate::schema::shared::FrontmatterFormat::Auto,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Array {
                            array: Box::new(crate::schema::shared::FieldTypeSerde::Scalar(
                                "String".into(),
                            )),
                        },
                        allowed: vec!["**".into()],
                        required: vec!["blog/**".into()],
                        nullable: false,
                        constraints: None,
                        preprocess: vec![],
                    },
                ],
                max_categories: None,
                min_category_repetition: None,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "mock".into(),
                name: "mock".into(),
                revision: None,
                dim: Some(256),
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            crate::step::has_violations(&output),
            "expected validation violations"
        );
    }

    #[tokio::test]
    async fn build_succeeds_with_new_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // File has title + author, but toml only declares title
        // author is a "new field" — informational, should not block build
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\nauthor: Alice\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let mut config = MdvsToml {
            default_output_format: None,
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: crate::schema::shared::FrontmatterFormat::Auto,
            },
            update: crate::schema::config::UpdateConfig {},
            check: None,
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![crate::schema::config::TomlField {
                    name: "title".into(),
                    field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                }],
                max_categories: None,
                min_category_repetition: None,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "mock".into(),
                name: "mock".into(),
                revision: None,
                dim: Some(256),
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            build: None,
            search: Some(SearchConfig {
                default_limit: 10,
                auto_update: false,
                auto_build: false,
                internal_prefix: String::new(),
                aliases: HashMap::new(),
            }),
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build should succeed despite unknown "author" field
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(
            !crate::step::has_failed(&output),
            "build should succeed with new fields: {output:#?}"
        );

        // Verify index was created
        assert!(tmp.path().join(".mdvs/index.lance").exists());
    }

    // ========================================================================
    // Incremental build integration tests
    // ========================================================================

    /// Read `file_id`→filename map and `chunk_id`→`file_id` map from the Lance index.
    async fn read_index_state(dir: &Path) -> (HashMap<String, String>, Vec<(String, String)>) {
        let backend = Backend::lance(dir);
        let file_index = backend.read_file_index().await.unwrap();
        let file_map: HashMap<String, String> = file_index
            .iter()
            .map(|e| (e.filename.clone(), e.file_id.clone()))
            .collect();
        let chunks = backend.read_chunk_rows().await.unwrap();
        let chunk_pairs: Vec<(String, String)> = chunks
            .iter()
            .map(|c| (c.chunk_id.clone(), c.file_id.clone()))
            .collect();
        (file_map, chunk_pairs)
    }

    #[tokio::test]
    async fn incremental_no_changes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path()).await;

        // Build again with no changes
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_after, chunks_after) = read_index_state(tmp.path()).await;

        // file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // chunk_ids preserved (same chunks carried forward)
        let old_chunk_ids: HashSet<&str> =
            chunks_before.iter().map(|(id, _)| id.as_str()).collect();
        let new_chunk_ids: HashSet<&str> = chunks_after.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn incremental_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path()).await;
        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: Third\ntags:\n  - new\ndraft: false\n---\n# Third\nNew post content.",
        )
        .unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_after, chunks_after) = read_index_state(tmp.path()).await;

        // Old file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // New file added
        assert!(files_after.contains_key("blog/post3.md"));
        assert_eq!(files_after.len(), 3);

        // Old chunks preserved, new chunks added
        for (chunk_id, _) in &chunks_before {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "old chunk {chunk_id} missing",
            );
        }
        assert!(chunks_after.len() > chunks_before.len());
    }

    #[tokio::test]
    async fn incremental_edited_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path()).await;
        let post1_id = files_before["blog/post1.md"].clone();
        let post2_id = files_before["blog/post2.md"].clone();

        // Chunks belonging to each file
        let post1_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        let post2_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post2_id)
            .map(|(cid, _)| cid.clone())
            .collect();

        // Edit post1's body (keep same frontmatter)
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nCompletely different body text.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_after, chunks_after) = read_index_state(tmp.path()).await;

        // file_ids preserved for both files
        assert_eq!(files_after["blog/post1.md"], post1_id);
        assert_eq!(files_after["blog/post2.md"], post2_id);

        // Counts: post1 re-embedded, post2 carried forward, nothing removed.
        let new_post1_chunk_count = chunks_after
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .count();
        let outcome = unwrap_build(&output);
        assert!(!outcome.full_rebuild);
        assert_eq!(outcome.files_total, files_after.len());
        assert_eq!(outcome.files_embedded, 1);
        assert_eq!(outcome.files_unchanged, 1);
        assert_eq!(outcome.files_removed, 0);
        assert_eq!(outcome.chunks_total, chunks_after.len());
        assert_eq!(outcome.chunks_embedded, new_post1_chunk_count);
        assert_eq!(outcome.chunks_unchanged, post2_chunks.len());
        assert_eq!(outcome.chunks_removed, 0);

        // post2 chunks preserved (unchanged file)
        for chunk_id in &post2_chunks {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "post2 chunk {chunk_id} should be preserved",
            );
        }
        // post1 chunks replaced (edited file — new chunk_ids)
        let new_post1_chunks: HashSet<String> = chunks_after
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        assert!(!new_post1_chunks.is_empty());
        for old_id in &post1_chunks {
            assert!(
                !new_post1_chunks.contains(old_id),
                "old chunk should be replaced"
            );
        }
    }

    #[tokio::test]
    async fn incremental_removed_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path()).await;
        assert_eq!(files_before.len(), 2);

        // Remove post2
        fs::remove_file(tmp.path().join("blog/post2.md")).unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_after, chunks_after) = read_index_state(tmp.path()).await;

        assert_eq!(files_after.len(), 1);
        assert!(files_after.contains_key("blog/post1.md"));
        assert!(!files_after.contains_key("blog/post2.md"));

        // No chunks referencing removed file
        let post2_id = &files_before["blog/post2.md"];
        assert!(!chunks_after.iter().any(|(_, fid)| fid == post2_id));

        // Counts: post1 carried forward, post2 and its chunks removed.
        let post2_chunk_count = chunks_before
            .iter()
            .filter(|(_, fid)| fid == post2_id)
            .count();
        let outcome = unwrap_build(&output);
        assert!(!outcome.full_rebuild);
        assert_eq!(outcome.files_total, files_after.len());
        assert_eq!(outcome.files_embedded, 0);
        assert_eq!(outcome.files_unchanged, files_after.len());
        assert_eq!(outcome.files_removed, 1);
        assert_eq!(outcome.chunks_total, chunks_after.len());
        assert_eq!(outcome.chunks_embedded, 0);
        assert_eq!(outcome.chunks_unchanged, chunks_after.len());
        assert_eq!(outcome.chunks_removed, post2_chunk_count);
    }

    #[tokio::test]
    async fn incremental_frontmatter_only_change() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (_, chunks_before) = read_index_state(tmp.path()).await;
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Change only frontmatter (add a tag), keep same body
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\n  - new-tag\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        ).unwrap();

        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (_, chunks_after) = read_index_state(tmp.path()).await;
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // Chunks preserved — body didn't change, no re-embedding
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn force_full_rebuild() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        let init_output = crate::cmd::init::run(
            tmp.path(),
            "**",
            InitOptions {
                scan: InitScanFlags {
                    ignore_bare_files: true,
                    ..Default::default()
                },
                ..Default::default()
            }, // verbose
            None,
            None,
        );
        assert!(!crate::step::has_failed(&init_output));

        // Build the index
        let output = run(tmp.path(), None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_before, chunks_before) = read_index_state(tmp.path()).await;
        let old_file_ids: HashSet<String> = files_before.values().cloned().collect();
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Force rebuild — should generate all new IDs
        let output = run(tmp.path(), None, None, None, true, true).await;
        assert!(!crate::step::has_failed(&output));

        let (files_after, chunks_after) = read_index_state(tmp.path()).await;
        let new_file_ids: HashSet<String> = files_after.values().cloned().collect();
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // All IDs should be different (new UUIDs)
        assert!(
            old_file_ids.is_disjoint(&new_file_ids),
            "force rebuild should generate new file_ids"
        );
        assert!(
            old_chunk_ids.is_disjoint(&new_chunk_ids),
            "force rebuild should generate new chunk_ids"
        );

        // Counts: every file and chunk re-embedded, nothing carried forward.
        let outcome = unwrap_build(&output);
        assert!(outcome.full_rebuild);
        assert_eq!(outcome.files_total, files_after.len());
        assert_eq!(outcome.files_embedded, files_after.len());
        assert_eq!(outcome.files_unchanged, 0);
        assert_eq!(outcome.files_removed, 0);
        assert_eq!(outcome.chunks_total, chunks_after.len());
        assert_eq!(outcome.chunks_embedded, chunks_after.len());
        assert_eq!(outcome.chunks_unchanged, 0);
        assert_eq!(outcome.chunks_removed, 0);
    }

    // ========================================================================
    // Categorical constraint integration tests
    // ========================================================================

    #[tokio::test]
    async fn build_succeeds_with_categorical_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let blog = tmp.path().join("blog");
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
                format!("---\nstatus: {status}\ntitle: Post {i}\n---\n# Post {i}\nBody text."),
            )
            .unwrap();
        }

        // Init with auto-build disabled, then verify categories inferred
        let init_step = crate::cmd::init::run(
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
        assert!(!crate::step::has_failed(&init_step));

        let toml = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let status = toml
            .fields
            .field
            .iter()
            .find(|f| f.name == "status")
            .unwrap();
        assert!(
            status.constraints.is_some(),
            "categories should be inferred on status"
        );

        // Build should succeed despite constraints in toml
        let build_step = run(tmp.path(), None, None, None, false, false).await;
        assert!(!crate::step::has_failed(&build_step));
        let result = unwrap_build(&build_step);
        assert_eq!(result.files_embedded, 9);
        assert!(tmp.path().join(".mdvs/index.lance").exists());
    }

    #[tokio::test]
    async fn build_aborts_on_invalid_category() {
        let tmp = tempfile::tempdir().unwrap();
        let blog = tmp.path().join("blog");
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
                format!("---\nstatus: {status}\ntitle: Post {i}\n---\n# Post {i}\nBody text."),
            )
            .unwrap();
        }

        // Init (infers categories on status)
        let init_step = crate::cmd::init::run(
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
        assert!(!crate::step::has_failed(&init_step));

        // Corrupt a file with an out-of-category value
        fs::write(
            blog.join("post0.md"),
            "---\nstatus: pending\ntitle: Post 0\n---\n# Post 0\nBody text.",
        )
        .unwrap();

        // Build should abort (build includes check internally)
        let build_step = run(tmp.path(), None, None, None, false, false).await;
        assert!(crate::step::has_failed(&build_step));
    }
}
