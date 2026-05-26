use crate::index::backend::{Backend, SearchMode};
use crate::index::embed::{Embedder, ModelConfig};
use crate::index::storage::BuildMetadata;
use crate::outcome::commands::SearchOutcome;
use crate::outcome::{
    EmbedQueryOutcome, ExecuteSearchOutcome, LoadModelOutcome, Outcome, ReadConfigOutcome,
    ReadIndexOutcome,
};
use crate::schema::config::MdvsToml;
use crate::step::{CommandResult, ErrorKind, StepEntry};
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

/// Index metadata, used for model mismatch check.
struct IndexData {
    metadata: BuildMetadata,
}

/// Validate --where clause for unmatched quotes.
fn validate_where_clause(w: &str) -> Result<(), String> {
    if w.chars().filter(|&c| c == '\'').count() % 2 != 0 {
        return Err(
            "unmatched single quote in --where clause — escape with '' (e.g. O''Brien)".into(),
        );
    }
    if w.chars().filter(|&c| c == '"').count() % 2 != 0 {
        return Err(
            "unmatched double quote in --where clause — escape with \"\" (e.g. \"\"field\"\")"
                .into(),
        );
    }
    Ok(())
}

/// Embed a query, search the index, and return ranked results.
#[instrument(name = "search", skip_all)]
#[allow(clippy::too_many_arguments)]
pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
    mode: SearchMode,
    no_update: bool,
    no_build: bool,
    _verbose: bool,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // 1. Read config — calls MdvsToml::read() + validate() directly
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
                Some(cfg)
            }
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::User,
                    format!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'"),
                    config_start.elapsed().as_millis() as u64,
                ));
                None
            }
        },
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                e.to_string(),
                config_start.elapsed().as_millis() as u64,
            ));
            None
        }
    };

    // Auto-build: run build core pipeline before searching if configured
    let mut build_embedder: Option<Embedder> = None;
    if let Some(ref mut cfg) = config {
        let should_build = !no_build && cfg.search.as_ref().is_some_and(|s| s.auto_build);
        if should_build {
            let build_no_update = no_update || !cfg.search.as_ref().is_some_and(|s| s.auto_update);
            let auto_update = !build_no_update && cfg.build.as_ref().is_some_and(|b| b.auto_update);

            // Fill missing build sections (embedding_model, chunking, search, build)
            crate::cmd::build::mutate_config(cfg, path, None, None, None, false);

            match crate::cmd::build::build_core(
                path,
                cfg,
                &config_path_buf,
                false,
                auto_update,
                &mut steps,
            )
            .await
            {
                Ok((_build_outcome, embedder)) => {
                    build_embedder = embedder;
                }
                Err(()) => {
                    return CommandResult::failed(
                        std::mem::take(&mut steps),
                        ErrorKind::User,
                        "auto-build failed".into(),
                        start,
                    );
                }
            }
        }
    }

    let embedding = config.as_ref().and_then(|c| c.embedding_model.as_ref());

    // 2. Read index — calls Backend methods directly
    let index_data = match &config {
        Some(_) => {
            let index_start = Instant::now();
            let backend = Backend::lance(path);
            if !backend.exists() {
                steps.push(StepEntry::ok(
                    Outcome::ReadIndex(ReadIndexOutcome {
                        exists: false,
                        files_indexed: 0,
                        chunks: 0,
                    }),
                    index_start.elapsed().as_millis() as u64,
                ));
                None
            } else {
                let build_meta = backend.read_metadata().await.ok().flatten();
                let idx_stats = backend.stats().await.ok().flatten();
                match (build_meta, idx_stats) {
                    (Some(metadata), Some(stats)) => {
                        steps.push(StepEntry::ok(
                            Outcome::ReadIndex(ReadIndexOutcome {
                                exists: true,
                                files_indexed: stats.files_indexed,
                                chunks: stats.chunks,
                            }),
                            index_start.elapsed().as_millis() as u64,
                        ));
                        Some(IndexData { metadata })
                    }
                    _ => {
                        steps.push(StepEntry::ok(
                            Outcome::ReadIndex(ReadIndexOutcome {
                                exists: false,
                                files_indexed: 0,
                                chunks: 0,
                            }),
                            index_start.elapsed().as_millis() as u64,
                        ));
                        None
                    }
                }
            }
        }
        None => {
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    };

    // Pre-checks before loading model
    let pre_check_error: Option<String> = match (config.as_ref(), embedding, index_data.as_ref()) {
        (None, _, _) => None,
        (_, None, _) => {
            Some("missing [embedding_model] in mdvs.toml (run `mdvs build` first)".to_string())
        }
        (_, _, None) => Some("index not found (run `mdvs build` first)".to_string()),
        (_, Some(emb), Some(data)) => {
            if data.metadata.embedding_model != *emb {
                Some(format!(
                    "model mismatch: config has '{}' (rev {:?}) but index was built with '{}' (rev {:?}) — run 'mdvs build' to rebuild",
                    emb.name,
                    emb.revision,
                    data.metadata.embedding_model.name,
                    data.metadata.embedding_model.revision,
                ))
            } else {
                None
            }
        }
    };

    // 3. Load model — calls ModelConfig::try_from() + Embedder::load() directly
    if let Some(msg) = pre_check_error {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
    }

    // 3. Load model (reuse from build if available).
    // The pre_check above ensures `embedding` is Some when we reach this
    // point; fall through to a step-level error if a future refactor
    // breaks that invariant.
    let Some(emb_config) = embedding else {
        steps.push(StepEntry::err(
            ErrorKind::Application,
            "internal: missing embedding config after pre-check passed".to_string(),
            0,
        ));
        return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
    };
    let embedder = if let Some(emb) = build_embedder {
        steps.push(StepEntry::ok(
            Outcome::LoadModel(LoadModelOutcome {
                model_name: emb_config.name.clone(),
                dimension: emb.dimension(),
            }),
            0, // already loaded during build
        ));
        emb
    } else {
        let model_start = Instant::now();
        match ModelConfig::try_from(emb_config) {
            Ok(mc) => match Embedder::load(&mc) {
                Ok(emb) => {
                    steps.push(StepEntry::ok(
                        Outcome::LoadModel(LoadModelOutcome {
                            model_name: emb_config.name.clone(),
                            dimension: emb.dimension(),
                        }),
                        model_start.elapsed().as_millis() as u64,
                    ));
                    emb
                }
                Err(e) => {
                    steps.push(StepEntry::err(
                        ErrorKind::Application,
                        e.to_string(),
                        model_start.elapsed().as_millis() as u64,
                    ));
                    return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
                }
            },
            Err(e) => {
                steps.push(StepEntry::err(
                    ErrorKind::Application,
                    e.to_string(),
                    model_start.elapsed().as_millis() as u64,
                ));
                return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
            }
        }
    };

    // 4. Embed query — calls embedder.embed() directly (infallible)
    let embed_start = Instant::now();
    let query_embedding = embedder.embed(query).await;
    steps.push(StepEntry::ok(
        Outcome::EmbedQuery(EmbedQueryOutcome {
            query: query.to_string(),
        }),
        embed_start.elapsed().as_millis() as u64,
    ));

    // 5. Execute search — calls backend.search() directly with quote validation.
    // Same invariant as `embedding` above: pre_check guarantees `config` is Some.
    let Some(cfg) = config.as_ref() else {
        steps.push(StepEntry::err(
            ErrorKind::Application,
            "internal: missing config after pre-check passed".to_string(),
            0,
        ));
        return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
    };
    let backend = Backend::lance(path);
    let (prefix, aliases) = match &cfg.search {
        Some(sc) => (sc.internal_prefix.as_str(), &sc.aliases),
        None => ("", &std::collections::HashMap::new()),
    };

    if let Some(w) = where_clause
        && let Err(msg) = validate_where_clause(w)
    {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
    }

    let search_start = Instant::now();
    let hits = match backend
        .search(
            query_embedding,
            query,
            mode,
            where_clause,
            limit,
            prefix,
            aliases,
        )
        .await
    {
        Ok(hits) => {
            steps.push(StepEntry::ok(
                Outcome::ExecuteSearch(ExecuteSearchOutcome { hits: hits.len() }),
                search_start.elapsed().as_millis() as u64,
            ));
            hits
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                search_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    };

    // chunk_text is populated by the backend from the persisted column.
    let model_name = emb_config.name.clone();
    CommandResult {
        steps,
        result: Ok(Outcome::Search(Box::new(SearchOutcome {
            query: query.to_string(),
            hits,
            model_name,
            limit,
        }))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::{Embedder, ModelConfig};
    use crate::outcome::commands::SearchOutcome;
    use crate::schema::config::{FieldsConfig, MdvsToml, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FrontmatterFormat, ScanConfig};
    use crate::step::StepError;
    use std::fs;

    fn unwrap_search(result: &CommandResult) -> &SearchOutcome {
        match &result.result {
            Ok(Outcome::Search(o)) => o,
            other => panic!("expected Ok(Search), got: {other:?}"),
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
            "---\ntitle: Rust Programming\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Rust Programming\nRust is a systems programming language focused on safety and performance.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: Cooking Recipes\ndraft: true\n---\n# Cooking Recipes\nDelicious pasta recipes for weeknight dinners.",
        )
        .unwrap();
    }

    fn write_config(dir: &Path, model_name: &str) {
        let mut config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: FrontmatterFormat::Auto,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: model_name.into(),
                revision: None,
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
                aliases: std::collections::HashMap::new(),
            }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    async fn init_and_build(dir: &Path) {
        let step = crate::cmd::init::run(dir, "**", false, false, true, false, false, None);
        assert!(!crate::step::has_failed(&step));
        let output = crate::cmd::build::run(dir, None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(
            tmp.path(),
            "test query",
            10,
            None,
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let output = run(
            tmp.path(),
            "test query",
            10,
            None,
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("index not found"));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "rust programming",
            10,
            None,
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(
            !crate::step::has_failed(&output),
            "search failed: {:?}",
            output
        );
        let result = unwrap_search(&output);
        assert_eq!(result.query, "rust programming");
        assert!(!result.model_name.is_empty());
        assert!(!result.hits.is_empty());
        assert!(result.hits[0].start_line.is_some());
        assert!(result.hits[0].end_line.is_some());
        // chunk_text always populated now (full outcome carries all data)
        assert!(result.hits[0].chunk_text.is_some());
    }

    #[tokio::test]
    async fn end_to_end_verbose() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "rust programming",
            10,
            None,
            SearchMode::Hybrid,
            true,
            true,
            true,
        )
        .await;
        assert!(!crate::step::has_failed(&output));
        let result = unwrap_search(&output);
        assert!(!result.hits.is_empty());
        assert!(result.hits[0].chunk_text.is_some());
    }

    #[tokio::test]
    async fn with_limit() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::lance(tmp.path());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("rust programming").await;

        let hits = backend
            .search(
                query_embedding,
                "rust programming",
                SearchMode::Semantic,
                None,
                1,
                "",
                &std::collections::HashMap::new(),
            )
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn with_where_clause() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::lance(tmp.path());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("cooking recipes").await;

        let hits = backend
            .search(
                query_embedding,
                "cooking recipes",
                SearchMode::Semantic,
                Some("draft = false"),
                10,
                "",
                &std::collections::HashMap::new(),
            )
            .await
            .unwrap();

        for hit in &hits {
            assert_ne!(hit.filename, "blog/post2.md");
        }
    }

    #[tokio::test]
    async fn model_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.embedding_model.as_mut().unwrap().name = "some-other-model".into();
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let output = run(
            tmp.path(),
            "test query",
            10,
            None,
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("model mismatch"));
    }

    #[tokio::test]
    async fn where_unmatched_single_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "test",
            10,
            Some("author = 'O'Brien'"),
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("unmatched single quote"));
    }

    #[tokio::test]
    async fn where_unmatched_double_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "test",
            10,
            Some("x = \"bad"),
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("unmatched double quote"));
    }

    #[tokio::test]
    async fn where_even_but_malformed_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "test",
            10,
            Some("author's name = O'Brien"),
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        assert!(crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn where_balanced_quotes_pass() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            "test",
            10,
            Some("title = 'O''Brien'"),
            SearchMode::Hybrid,
            true,
            true,
            false,
        )
        .await;
        // Should not fail with quote parity error
        if let Err(e) = &output.result {
            assert!(
                !e.message.contains("unmatched"),
                "balanced quotes should not trigger parity check"
            );
        }
    }

    // --- Unit tests for validate_where_clause ---

    #[test]
    fn validate_where_valid() {
        assert!(validate_where_clause("draft = false").is_ok());
    }

    #[test]
    fn validate_where_empty() {
        assert!(validate_where_clause("").is_ok());
    }

    #[test]
    fn validate_where_unmatched_single() {
        assert!(validate_where_clause("name = 'O'Brien'").is_err());
    }

    #[test]
    fn validate_where_unmatched_double() {
        assert!(validate_where_clause("x = \"bad").is_err());
    }

    #[test]
    fn validate_where_balanced_quotes() {
        assert!(validate_where_clause("name = 'O''Brien'").is_ok());
    }

    // ========================================================================
    // Integration tests — real embedder + Lance index (TODO-0016 wave 2).
    // A richer vault exercises all search modes and --where operator families.
    // ========================================================================

    /// Six-file vault with varied frontmatter (String/Integer/Boolean/Date/
    /// Array/nested Float) and distinctive body keywords. `rust.md` has a long
    /// body so it splits into multiple chunks (dedupe coverage).
    fn create_rich_vault(dir: &Path) {
        let blog = dir.join("blog");
        let notes = dir.join("notes");
        fs::create_dir_all(&blog).unwrap();
        fs::create_dir_all(&notes).unwrap();

        let long_body: String = "Rust gives strong guarantees about memory without a \
            garbage collector. Ownership and borrowing are checked at compile time, so \
            whole classes of bugs simply cannot happen. "
            .repeat(20);
        fs::write(
            blog.join("rust.md"),
            format!(
                "---\ntitle: Rust Programming\nstatus: active\nrating: 5\ndraft: false\n\
                 published: 2024-01-15\ntags:\n  - rust\n  - systems\n---\n# Rust\n{long_body}"
            ),
        )
        .unwrap();

        fs::write(
            blog.join("cooking.md"),
            "---\ntitle: Cooking Pasta\nstatus: archived\nrating: 2\ndraft: true\n\
             published: 2023-06-15\ntags:\n  - food\n---\n# Cooking\nDelicious pasta recipes for weeknight dinners.",
        )
        .unwrap();

        fs::write(
            notes.join("photonics.md"),
            "---\ntitle: Photonics Calibration\nstatus: active\nrating: 4\ndraft: false\n\
             published: 2024-03-10\ntags:\n  - optics\ncalibration:\n  baseline:\n    wavelength: 850.0\n---\n\
             # Photonics\nThe sensor wavelength drifts over time and requires periodic recalibration of each pixel.",
        )
        .unwrap();

        fs::write(
            notes.join("draftpost.md"),
            "---\ntitle: Draft Ideas\nstatus: draft\nrating: 3\ndraft: true\n\
             published: 2024-05-01\ntags:\n  - misc\n---\n# Ideas\nA scratch list of half-formed ideas.",
        )
        .unwrap();

        fs::write(
            notes.join("review.md"),
            "---\ntitle: Annual Review\nstatus: active\nrating: 4\ndraft: false\n\
             published: 2024-02-20\ntags:\n  - meta\n---\n# Review\nA look back at the year of work.",
        )
        .unwrap();

        fs::write(
            blog.join("archive.md"),
            "---\ntitle: Old Archive\nstatus: archived\nrating: 1\ndraft: true\n\
             published: 2022-11-05\ntags:\n  - old\n---\n# Archive\nLegacy content kept for posterity.",
        )
        .unwrap();
    }

    /// Build the rich vault once, then return the filenames of the hits for a
    /// query/mode/where, sorted for stable assertions.
    async fn search_files(
        dir: &Path,
        query: &str,
        mode: SearchMode,
        where_clause: Option<&str>,
    ) -> Vec<String> {
        let result = run(dir, query, 50, where_clause, mode, true, true, false).await;
        assert!(
            !crate::step::has_failed(&result),
            "search failed: {result:#?}"
        );
        let mut files: Vec<String> = unwrap_search(&result)
            .hits
            .iter()
            .map(|h| h.filename.clone())
            .collect();
        files.sort();
        files
    }

    fn ends_with(files: &[String], suffix: &str) -> bool {
        files.iter().any(|f| f.ends_with(suffix))
    }

    #[tokio::test]
    async fn integration_modes() {
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // Semantic: a paraphrase ("memory safety guarantees") should surface the
        // Rust doc even though those exact words aren't all present.
        let sem = search_files(
            tmp.path(),
            "memory safety guarantees",
            SearchMode::Semantic,
            None,
        )
        .await;
        assert!(
            ends_with(&sem, "rust.md"),
            "semantic should find rust.md: {sem:?}"
        );

        // Fulltext: the exact keyword "wavelength" appears only in photonics.md.
        let ft = search_files(tmp.path(), "wavelength", SearchMode::Fulltext, None).await;
        assert!(
            ends_with(&ft, "photonics.md"),
            "fulltext should find photonics.md: {ft:?}"
        );
        assert!(
            !ends_with(&ft, "rust.md"),
            "fulltext 'wavelength' should not match rust.md: {ft:?}"
        );

        // Hybrid (default) returns a non-empty fused ranking.
        let hy = search_files(tmp.path(), "calibration drift", SearchMode::Hybrid, None).await;
        assert!(!hy.is_empty(), "hybrid should return results");
    }

    #[tokio::test]
    async fn integration_where_operators() {
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;
        let q = "content";

        // String equality
        let active = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("status = 'active'"),
        )
        .await;
        assert!(
            active.iter().all(|f| !f.ends_with("cooking.md")),
            "active filter excludes archived: {active:?}"
        );
        assert!(ends_with(&active, "rust.md") && ends_with(&active, "photonics.md"));

        // Integer comparisons
        let hi = search_files(tmp.path(), q, SearchMode::Semantic, Some("rating >= 4")).await;
        assert!(
            !hi.is_empty()
                && hi
                    .iter()
                    .all(|f| !f.ends_with("cooking.md") && !f.ends_with("archive.md"))
        );
        let lo = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("rating BETWEEN 1 AND 2"),
        )
        .await;
        assert!(ends_with(&lo, "cooking.md") && ends_with(&lo, "archive.md"));
        let in_list = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("rating IN (1, 5)"),
        )
        .await;
        assert!(ends_with(&in_list, "rust.md") && ends_with(&in_list, "archive.md"));

        // Boolean
        let published =
            search_files(tmp.path(), q, SearchMode::Semantic, Some("draft = false")).await;
        assert!(
            published
                .iter()
                .all(|f| !f.ends_with("cooking.md") && !f.ends_with("draftpost.md"))
        );

        // Array membership
        let rusty = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("array_has(tags, 'rust')"),
        )
        .await;
        assert_eq!(rusty.len(), 1);
        assert!(ends_with(&rusty, "rust.md"));

        // LIKE on a string field
        let titled = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("title LIKE 'Rust%'"),
        )
        .await;
        assert!(ends_with(&titled, "rust.md") && titled.len() == 1);

        // Date literal comparison
        let recent = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("published >= date '2024-01-01'"),
        )
        .await;
        assert!(
            recent
                .iter()
                .all(|f| !f.ends_with("cooking.md") && !f.ends_with("archive.md"))
        );

        // Nested dotted struct access
        let nested = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("calibration.baseline.wavelength > 800"),
        )
        .await;
        assert_eq!(nested.len(), 1);
        assert!(ends_with(&nested, "photonics.md"));

        // AND composition
        let combo = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("status = 'active' AND rating >= 5"),
        )
        .await;
        assert_eq!(combo.len(), 1);
        assert!(ends_with(&combo, "rust.md"));

        // Internal column filter (filepath stays top-level, not data-prefixed)
        let blog_only = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("filepath LIKE 'blog/%'"),
        )
        .await;
        assert!(blog_only.iter().all(|f| f.starts_with("blog/")));
        assert!(!blog_only.is_empty());

        // A filter that matches nothing returns zero hits (not an error)
        let none = search_files(tmp.path(), q, SearchMode::Semantic, Some("rating > 100")).await;
        assert!(none.is_empty());

        // Filtering reduces the result set vs. no filter
        let all = search_files(tmp.path(), q, SearchMode::Semantic, None).await;
        let filtered = search_files(
            tmp.path(),
            q,
            SearchMode::Semantic,
            Some("status = 'archived'"),
        )
        .await;
        assert!(filtered.len() < all.len() && !filtered.is_empty());
    }

    #[tokio::test]
    async fn integration_dedupe_limit_and_snippet() {
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // rust.md has a long, multi-chunk body — it must appear at most once.
        let files = search_files(
            tmp.path(),
            "rust ownership borrowing",
            SearchMode::Semantic,
            None,
        )
        .await;
        let rust_count = files.iter().filter(|f| f.ends_with("rust.md")).count();
        assert_eq!(
            rust_count, 1,
            "multi-chunk file should be deduped to one hit"
        );

        // Limit is respected.
        let result = run(
            tmp.path(),
            "content",
            2,
            None,
            SearchMode::Semantic,
            true,
            true,
            false,
        )
        .await;
        assert!(!crate::step::has_failed(&result));
        assert!(unwrap_search(&result).hits.len() <= 2);

        // The snippet (chunk_text) is populated from the persisted column.
        let result = run(
            tmp.path(),
            "wavelength",
            1,
            None,
            SearchMode::Fulltext,
            true,
            true,
            false,
        )
        .await;
        let hits = &unwrap_search(&result).hits;
        assert!(!hits.is_empty());
        assert!(hits[0].chunk_text.as_ref().is_some_and(|t| !t.is_empty()));
    }

    #[tokio::test]
    async fn integration_hybrid_zero_results_is_empty_not_error() {
        // Regression: a hybrid query whose --where matches nothing returns an
        // empty batch with no projected columns; the reader must yield zero
        // hits, not "missing column file_id".
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;
        for mode in [
            SearchMode::Hybrid,
            SearchMode::Fulltext,
            SearchMode::Semantic,
        ] {
            let hits = search_files(tmp.path(), "content", mode, Some("rating > 1000")).await;
            assert!(
                hits.is_empty(),
                "{mode:?} zero-match should be empty: {hits:?}"
            );
        }
    }

    #[tokio::test]
    async fn integration_scalar_functions_in_where() {
        // Scalar functions over frontmatter fields work: the translator leaves
        // the function name and prefixes only its column arguments.
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // lower() — case-folded match against the lowercase status values.
        let lowered = search_files(
            tmp.path(),
            "content",
            SearchMode::Semantic,
            Some("lower(status) = 'active'"),
        )
        .await;
        assert!(ends_with(&lowered, "rust.md") && ends_with(&lowered, "photonics.md"));

        // length() — titles longer than 6 chars (excludes none of the long ones).
        let lengthy = search_files(
            tmp.path(),
            "content",
            SearchMode::Semantic,
            Some("length(title) > 6"),
        )
        .await;
        assert!(!lengthy.is_empty());

        // arithmetic on an integer field
        let arith = search_files(
            tmp.path(),
            "content",
            SearchMode::Semantic,
            Some("rating + 1 >= 6"),
        )
        .await;
        assert!(ends_with(&arith, "rust.md"));
    }

    #[tokio::test]
    async fn integration_limit_zero_is_empty_not_error() {
        // `--limit 0` must return zero hits gracefully across all modes
        // (LanceDB rejects a zero `k` internally).
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;
        for mode in [
            SearchMode::Hybrid,
            SearchMode::Fulltext,
            SearchMode::Semantic,
        ] {
            let result = run(tmp.path(), "content", 0, None, mode, true, true, false).await;
            assert!(
                !crate::step::has_failed(&result),
                "{mode:?} limit 0 should not fail"
            );
            assert!(unwrap_search(&result).hits.is_empty());
        }
    }

    #[tokio::test]
    async fn integration_array_float_filter_errors_not_panics() {
        // A --where on an Array(Float) field used to panic/hang in lance-encoding
        // (TODO-0159). The translator now refuses the reference with a clean
        // error across all modes.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();
        fs::write(
            tmp.path().join("notes/a.md"),
            "---\ntitle: A\nmeasurement_values: [0.5, 0.6, 0.7]\n---\n# A\nsome body content for chunking and embedding.",
        )
        .unwrap();
        fs::write(
            tmp.path().join("notes/b.md"),
            "---\ntitle: B\n---\n# B\nanother document with different content.",
        )
        .unwrap();
        init_and_build(tmp.path()).await;

        for mode in [
            SearchMode::Hybrid,
            SearchMode::Fulltext,
            SearchMode::Semantic,
        ] {
            let result = run(
                tmp.path(),
                "content",
                10,
                Some("measurement_values IS NOT NULL"),
                mode,
                true,
                true,
                false,
            )
            .await;
            assert!(
                crate::step::has_failed(&result),
                "{mode:?} should fail with a clean error"
            );
            let dump = format!("{result:?}");
            assert!(
                dump.contains("Array(Float)"),
                "{mode:?} should report the Array(Float) message: {dump}"
            );
        }
    }

    #[tokio::test]
    async fn integration_collision_surfaces_error() {
        // A frontmatter field named like an internal column, with no aliasing,
        // must surface the translator's collision error rather than silently
        // shadowing the field.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("note.md"),
            "---\ntitle: Note\nfile_id: abc\n---\n# Note\nbody text here",
        )
        .unwrap();
        init_and_build(tmp.path()).await;

        let result = run(
            tmp.path(),
            "note",
            10,
            Some("file_id = 'abc'"),
            SearchMode::Semantic,
            true,
            true,
            false,
        )
        .await;
        assert!(
            crate::step::has_failed(&result),
            "collision should fail the search"
        );
    }
}
