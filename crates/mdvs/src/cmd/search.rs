use crate::cmd::build::{build_core, mutate_config};
use crate::cmd::steps::{load_model_step, read_config_step, read_index_step};
use crate::index::backend::{Backend, SearchMode, SearchQuery, SearchResults, WhereNaming};
use crate::index::embed::Embedder;
use crate::index::storage::BuildMetadata;
use crate::outcome::commands::SearchOutcome;
use crate::outcome::{EmbedQueryOutcome, ExecuteSearchOutcome, LoadModelOutcome, Outcome};
use crate::schema::config::MdvsToml;
use crate::schema::shared::EmbeddingModelConfig;
use crate::step::{CommandResult, ErrorKind, StepEntry, elapsed_ms};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;
use tracing::instrument;

/// Flags controlling the automatic build that `search` may run first.
#[derive(Debug, Default, Clone, Copy)]
pub struct SearchOptions {
    /// Skip auto-updating the schema with newly seen fields during the
    /// automatic build.
    pub no_update: bool,
    /// Skip the automatic build and search the index as it is.
    pub no_build: bool,
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
pub async fn run(path: &Path, query: SearchQuery<'_>, opts: SearchOptions) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    let Ok((mut config, config_path)) = read_config_step(path, &mut steps) else {
        return CommandResult::failed_from_steps(steps, start);
    };

    let Ok(build_embedder) =
        auto_build_step(path, &mut config, &config_path, opts, &mut steps).await
    else {
        return CommandResult::failed(steps, ErrorKind::User, "auto-build failed".into(), start);
    };

    let backend = Backend::lance(path);
    let index = read_index_step(&backend, &mut steps).await;

    let emb_config = match pre_check(
        config.embedding_model.as_ref(),
        index.as_ref().map(|(metadata, _)| metadata),
    ) {
        Ok(emb_config) => emb_config,
        Err(msg) => {
            steps.push(StepEntry::err(ErrorKind::User, msg, 0));
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    // Fulltext mode is BM25-only: the embedding is never read by the backend,
    // so skip the model load and the query-embedding step entirely.
    let query_embedding = if query.mode == SearchMode::Fulltext {
        None
    } else {
        let Ok(embedder) = resolve_embedder(emb_config, build_embedder, &mut steps) else {
            return CommandResult::failed_from_steps(steps, start);
        };
        Some(embed_query_step(&embedder, query.text, &mut steps).await)
    };

    if let Some(w) = query.where_clause
        && let Err(msg) = validate_where_clause(w)
    {
        steps.push(StepEntry::err(ErrorKind::User, msg, 0));
        return CommandResult::failed_from_steps(steps, start);
    }

    let empty_aliases = HashMap::new();
    let naming = match &config.search {
        Some(sc) => WhereNaming {
            internal_prefix: sc.internal_prefix.as_str(),
            aliases: &sc.aliases,
        },
        None => WhereNaming {
            internal_prefix: "",
            aliases: &empty_aliases,
        },
    };

    let Ok(results) =
        execute_search_step(&backend, &query, query_embedding, &naming, &mut steps).await
    else {
        return CommandResult::failed_from_steps(steps, start);
    };

    // chunk_text is populated by the backend from the persisted column.
    CommandResult {
        steps,
        result: Ok(Outcome::Search(Box::new(SearchOutcome {
            query: query.text.to_string(),
            hits: results.hits,
            model_name: emb_config.name.clone(),
            limit: query.limit,
            where_rewrites: results.where_rewrites,
        }))),
        elapsed_ms: elapsed_ms(start),
    }
}

/// Run the build pipeline before searching, when `[search].auto_build` is on
/// and `--no-build` was not passed.
///
/// Fills any missing build sections of `config` first. Returns the embedder
/// the build loaded, if any, so the search can reuse it. `Ok(None)` also
/// covers the case where no build ran.
async fn auto_build_step(
    path: &Path,
    config: &mut MdvsToml,
    config_path: &Path,
    opts: SearchOptions,
    steps: &mut Vec<StepEntry>,
) -> Result<Option<Embedder>, ()> {
    let search = config.search.as_ref();
    if opts.no_build || !search.is_some_and(|s| s.auto_build) {
        return Ok(None);
    }
    let build_no_update = opts.no_update || !search.is_some_and(|s| s.auto_update);
    let auto_update = !build_no_update && config.build.as_ref().is_some_and(|b| b.auto_update);

    // Fill missing build sections (embedding_model, chunking, search, build)
    mutate_config(config, path, None, None, None, false);

    let (_build_outcome, embedder) =
        build_core(path, config, config_path, false, auto_update, steps).await?;
    Ok(embedder)
}

/// Check that the config names an embedding model, that an index exists, and
/// that the index was built with that model.
///
/// Returns the configured model, or the message for the failure step.
fn pre_check<'a>(
    embedding: Option<&'a EmbeddingModelConfig>,
    index: Option<&BuildMetadata>,
) -> Result<&'a EmbeddingModelConfig, String> {
    match (embedding, index) {
        (None, _) => {
            Err("missing [embedding_model] in mdvs.toml (run `mdvs build` first)".to_string())
        }
        (_, None) => Err("index not found (run `mdvs build` first)".to_string()),
        (Some(emb), Some(metadata)) => {
            if metadata.embedding_model == *emb {
                Ok(emb)
            } else {
                Err(format!(
                    "model mismatch: config has '{}' (rev {:?}) but index was built with '{}' (rev {:?}) — run 'mdvs build' to rebuild",
                    emb.name,
                    emb.revision,
                    metadata.embedding_model.name,
                    metadata.embedding_model.revision,
                ))
            }
        }
    }
}

/// Reuse the embedder the automatic build loaded, or load one.
///
/// A reused embedder is still reported as a model-load step, with zero
/// elapsed time since the load happened during the build.
fn resolve_embedder(
    emb_config: &EmbeddingModelConfig,
    build_embedder: Option<Embedder>,
    steps: &mut Vec<StepEntry>,
) -> Result<Embedder, ()> {
    let Some(embedder) = build_embedder else {
        return load_model_step(emb_config, steps);
    };
    steps.push(StepEntry::ok(
        Outcome::LoadModel(LoadModelOutcome {
            model_name: emb_config.name.clone(),
            dimension: embedder.dimension(),
        }),
        0, // already loaded during build
    ));
    Ok(embedder)
}

/// Embed the query text, timed as one step. Embedding cannot fail.
async fn embed_query_step(embedder: &Embedder, text: &str, steps: &mut Vec<StepEntry>) -> Vec<f32> {
    let embed_start = Instant::now();
    let embedding = embedder.embed(text).await;
    steps.push(StepEntry::ok(
        Outcome::EmbedQuery(EmbedQueryOutcome {
            query: text.to_string(),
        }),
        elapsed_ms(embed_start),
    ));
    embedding
}

/// Run the search against the index, timed as one step.
async fn execute_search_step(
    backend: &Backend,
    query: &SearchQuery<'_>,
    query_embedding: Option<Vec<f32>>,
    naming: &WhereNaming<'_>,
    steps: &mut Vec<StepEntry>,
) -> Result<SearchResults, ()> {
    let search_start = Instant::now();
    match backend.search(query, query_embedding, naming).await {
        Ok(results) => {
            steps.push(StepEntry::ok(
                Outcome::ExecuteSearch(ExecuteSearchOutcome {
                    hits: results.hits.len(),
                }),
                elapsed_ms(search_start),
            ));
            Ok(results)
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(search_start),
            ));
            Err(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::init::{InitOptions, InitScanFlags};
    use crate::index::embed::{Embedder, ModelConfig};
    use crate::outcome::commands::SearchOutcome;
    use crate::schema::config::{FieldsConfig, MdvsToml, SearchConfig, UpdateConfig};
    use crate::schema::shared::{
        ChunkingConfig, EmbeddingModelConfig, FrontmatterFormat, ScanConfig,
    };
    use crate::step::{ProcessStep, StepError};
    use std::fs;
    use tempfile::TempDir;

    /// Search the index as it is: no automatic build, no schema update.
    const NO_AUTO: SearchOptions = SearchOptions {
        no_update: true,
        no_build: true,
    };

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
            default_output_format: None,
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
                max_categories: None,
                min_category_repetition: None,
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "mock".into(),
                name: model_name.into(),
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
                aliases: std::collections::HashMap::new(),
            }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    /// Initialise `dir` as a project that embeds with the mock model, without
    /// building an index.
    fn init_with_mock_embedder(dir: &Path) {
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
        swap_to_mock_embedder(dir);
    }

    async fn init_and_build(dir: &Path) {
        init_with_mock_embedder(dir);
        let output = crate::cmd::build::run(dir, None, None, None, false, true).await;
        assert!(!crate::step::has_failed(&output));
    }

    fn swap_to_mock_embedder(dir: &Path) {
        let mut config = MdvsToml::read(&dir.join("mdvs.toml")).unwrap();
        config.embedding_model = Some(EmbeddingModelConfig {
            provider: "mock".into(),
            name: "mock".into(),
            revision: None,
            dim: Some(256),
        });
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    /// The completed outcome of a step, or `None` for a failed or skipped one.
    fn completed(step: &StepEntry) -> Option<&ProcessStep> {
        match step {
            StepEntry::Completed(ps) => Some(ps),
            StepEntry::Failed(_) | StepEntry::Skipped => None,
        }
    }

    #[tokio::test]
    async fn auto_build_writes_index_then_search_loads_model_embeds_and_executes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_with_mock_embedder(tmp.path());

        let output = run(
            tmp.path(),
            SearchQuery {
                text: "rust programming",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Hybrid,
            },
            SearchOptions::default(),
        )
        .await;
        assert!(
            !crate::step::has_failed(&output),
            "search failed: {output:?}"
        );
        assert!(!unwrap_search(&output).hits.is_empty());

        let steps: Vec<Option<&ProcessStep>> = output.steps.iter().map(completed).collect();
        assert!(
            matches!(steps.first(), Some(Some(ps)) if matches!(ps.outcome, Outcome::ReadConfig(_))),
            "first step should read the config: {steps:?}"
        );

        // The build ran (it wrote the index) before the search read it.
        let read_index = steps
            .iter()
            .position(|s| s.is_some_and(|ps| matches!(ps.outcome, Outcome::ReadIndex(_))))
            .expect("a read-index step");
        assert!(
            steps[..read_index]
                .iter()
                .any(|s| s.is_some_and(|ps| matches!(ps.outcome, Outcome::WriteIndex(_)))),
            "build steps should precede the index read: {steps:?}"
        );

        // After the index read: the model load, the query embedding, then the
        // search itself. The zero elapsed time on the load step is consistent
        // with reusing the build's model, but the mock embedder loads in well
        // under a millisecond, so reuse and reload are not distinguishable here.
        let tail = &steps[read_index + 1..];
        assert!(
            matches!(
                tail,
                [Some(load), Some(embed), Some(search)]
                    if matches!(load.outcome, Outcome::LoadModel(_))
                        && load.elapsed_ms == 0
                        && matches!(embed.outcome, Outcome::EmbedQuery(_))
                        && matches!(search.outcome, Outcome::ExecuteSearch(_))
            ),
            "expected LoadModel (0 ms), EmbedQuery, ExecuteSearch after ReadIndex: {tail:?}"
        );
    }

    /// Chunk size recorded in hand-built index metadata; `pre_check` ignores it.
    const TEST_CHUNK_SIZE: usize = 1024;

    /// A mock-provider model config with the given name.
    fn mock_model(name: &str) -> EmbeddingModelConfig {
        EmbeddingModelConfig {
            provider: "mock".into(),
            name: name.into(),
            revision: None,
            dim: None,
        }
    }

    /// Index metadata recording a build with `model`.
    fn built_with(model: EmbeddingModelConfig) -> BuildMetadata {
        BuildMetadata {
            embedding_model: model,
            chunking: ChunkingConfig {
                max_chunk_size: TEST_CHUNK_SIZE,
            },
            glob: "**".into(),
            built_at: String::new(),
            schema_hash: String::new(),
        }
    }

    #[test]
    fn pre_check_reports_missing_model_config() {
        let metadata = built_with(mock_model("mock"));
        let err = pre_check(None, Some(&metadata)).unwrap_err();
        assert_eq!(
            err,
            "missing [embedding_model] in mdvs.toml (run `mdvs build` first)"
        );
    }

    #[test]
    fn pre_check_missing_model_config_wins_over_missing_index() {
        let err = pre_check(None, None).unwrap_err();
        assert_eq!(
            err,
            "missing [embedding_model] in mdvs.toml (run `mdvs build` first)"
        );
    }

    #[test]
    fn pre_check_reports_missing_index() {
        let model = mock_model("mock");
        let err = pre_check(Some(&model), None).unwrap_err();
        assert_eq!(err, "index not found (run `mdvs build` first)");
    }

    #[test]
    fn pre_check_reports_model_mismatch() {
        let model = mock_model("mock");
        let metadata = built_with(mock_model("other"));
        let err = pre_check(Some(&model), Some(&metadata)).unwrap_err();
        assert_eq!(
            err,
            "model mismatch: config has 'mock' (rev None) but index was built with 'other' (rev None) — run 'mdvs build' to rebuild"
        );
    }

    #[test]
    fn pre_check_accepts_matching_model() {
        let model = mock_model("mock");
        let metadata = built_with(mock_model("mock"));
        assert_eq!(pre_check(Some(&model), Some(&metadata)), Ok(&model));
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(
            tmp.path(),
            SearchQuery {
                text: "test query",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "test query",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "rust programming",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
        )
        .await;
        assert!(
            !crate::step::has_failed(&output),
            "search failed: {output:?}"
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
    async fn fulltext_skips_model_load_and_embed_query() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(
            tmp.path(),
            SearchQuery {
                text: "rust",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Fulltext,
            },
            NO_AUTO,
        )
        .await;
        assert!(
            !crate::step::has_failed(&output),
            "search failed: {output:?}"
        );
        let result = unwrap_search(&output);
        assert!(!result.hits.is_empty());

        // BM25 only — model load + query embed steps must NOT appear
        for step in &output.steps {
            if let crate::step::StepEntry::Completed(ps) = step {
                assert!(
                    !matches!(ps.outcome, Outcome::LoadModel(_)),
                    "fulltext should not load the embedding model"
                );
                assert!(
                    !matches!(ps.outcome, Outcome::EmbedQuery(_)),
                    "fulltext should not embed the query"
                );
            }
        }
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
                &SearchQuery {
                    text: "rust programming",
                    limit: 1,
                    where_clause: None,
                    mode: SearchMode::Semantic,
                },
                Some(query_embedding),
                &WhereNaming {
                    internal_prefix: "",
                    aliases: &std::collections::HashMap::new(),
                },
            )
            .await
            .unwrap();
        assert_eq!(hits.hits.len(), 1);
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
                &SearchQuery {
                    text: "cooking recipes",
                    limit: 10,
                    where_clause: Some("draft = false"),
                    mode: SearchMode::Semantic,
                },
                Some(query_embedding),
                &WhereNaming {
                    internal_prefix: "",
                    aliases: &std::collections::HashMap::new(),
                },
            )
            .await
            .unwrap();

        for hit in &hits.hits {
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
            SearchQuery {
                text: "test query",
                limit: 10,
                where_clause: None,
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "test",
                limit: 10,
                where_clause: Some("author = 'O'Brien'"),
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "test",
                limit: 10,
                where_clause: Some("x = \"bad"),
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "test",
                limit: 10,
                where_clause: Some("author's name = O'Brien"),
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
            SearchQuery {
                text: "test",
                limit: 10,
                where_clause: Some("title = 'O''Brien'"),
                mode: SearchMode::Hybrid,
            },
            NO_AUTO,
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
        let result = run(
            dir,
            SearchQuery {
                text: query,
                limit: 50,
                where_clause,
                mode,
            },
            NO_AUTO,
        )
        .await;
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

    /// Expected outcome of a `--where` filter over the rich vault. Files are
    /// matched by path suffix.
    enum WhereExpect {
        /// The hits are exactly these files.
        Only(&'static [&'static str]),
        /// At least one hit; all of `include` appear and none of `exclude`.
        Filtered {
            include: &'static [&'static str],
            exclude: &'static [&'static str],
        },
        /// At least one hit, and every hit's path starts with this prefix.
        UnderPrefix(&'static str),
        /// No hits, and no error.
        Empty,
        /// At least one hit, but fewer than the unfiltered search returns.
        NarrowerThanUnfiltered,
    }

    /// Semantic search for a fixed query over the rich vault, with an optional
    /// `--where` filter; returns the sorted hit filenames.
    async fn where_files(tmp: &TempDir, where_clause: Option<&str>) -> Vec<String> {
        search_files(tmp.path(), "content", SearchMode::Semantic, where_clause).await
    }

    #[tokio::test]
    async fn integration_where_operators() {
        let tmp = tempfile::tempdir().unwrap();
        create_rich_vault(tmp.path());
        init_and_build(tmp.path()).await;
        let unfiltered = where_files(&tmp, None).await;

        let cases = [
            // String equality
            (
                "status = 'active'",
                WhereExpect::Filtered {
                    include: &["rust.md", "photonics.md"],
                    exclude: &["cooking.md"],
                },
            ),
            // Integer comparisons
            (
                "rating >= 4",
                WhereExpect::Filtered {
                    include: &[],
                    exclude: &["cooking.md", "archive.md"],
                },
            ),
            (
                "rating BETWEEN 1 AND 2",
                WhereExpect::Only(&["cooking.md", "archive.md"]),
            ),
            (
                "rating IN (1, 5)",
                WhereExpect::Only(&["rust.md", "archive.md"]),
            ),
            // Boolean
            (
                "draft = false",
                WhereExpect::Filtered {
                    include: &[],
                    exclude: &["cooking.md", "draftpost.md"],
                },
            ),
            // Array membership
            ("array_has(tags, 'rust')", WhereExpect::Only(&["rust.md"])),
            // LIKE on a string field
            ("title LIKE 'Rust%'", WhereExpect::Only(&["rust.md"])),
            // Date literal comparison
            (
                "published >= date '2024-01-01'",
                WhereExpect::Filtered {
                    include: &[],
                    exclude: &["cooking.md", "archive.md"],
                },
            ),
            // Nested dotted struct access
            (
                "calibration.baseline.wavelength > 800",
                WhereExpect::Only(&["photonics.md"]),
            ),
            // AND composition
            (
                "status = 'active' AND rating >= 5",
                WhereExpect::Only(&["rust.md"]),
            ),
            // Internal column filter (filepath stays top-level, not data-prefixed)
            ("filepath LIKE 'blog/%'", WhereExpect::UnderPrefix("blog/")),
            // A filter that matches nothing returns zero hits (not an error)
            ("rating > 100", WhereExpect::Empty),
            // Filtering reduces the result set vs. no filter
            ("status = 'archived'", WhereExpect::NarrowerThanUnfiltered),
        ];

        for (where_clause, expect) in cases {
            let files = where_files(&tmp, Some(where_clause)).await;
            let ctx = format!("{where_clause}: {files:?}");
            match expect {
                WhereExpect::Only(expected) => {
                    assert_eq!(files.len(), expected.len(), "{ctx}");
                    assert!(expected.iter().all(|e| ends_with(&files, e)), "{ctx}");
                }
                WhereExpect::Filtered { include, exclude } => {
                    assert!(!files.is_empty(), "{ctx}");
                    assert!(include.iter().all(|e| ends_with(&files, e)), "{ctx}");
                    assert!(!exclude.iter().any(|e| ends_with(&files, e)), "{ctx}");
                }
                WhereExpect::UnderPrefix(prefix) => {
                    assert!(!files.is_empty(), "{ctx}");
                    assert!(files.iter().all(|f| f.starts_with(prefix)), "{ctx}");
                }
                WhereExpect::Empty => assert!(files.is_empty(), "{ctx}"),
                WhereExpect::NarrowerThanUnfiltered => {
                    assert!(!files.is_empty(), "{ctx}");
                    assert!(files.len() < unfiltered.len(), "{ctx}");
                }
            }
        }
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
            SearchQuery {
                text: "content",
                limit: 2,
                where_clause: None,
                mode: SearchMode::Semantic,
            },
            NO_AUTO,
        )
        .await;
        assert!(!crate::step::has_failed(&result));
        assert!(unwrap_search(&result).hits.len() <= 2);

        // The snippet (chunk_text) is populated from the persisted column.
        let result = run(
            tmp.path(),
            SearchQuery {
                text: "wavelength",
                limit: 1,
                where_clause: None,
                mode: SearchMode::Fulltext,
            },
            NO_AUTO,
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
            let result = run(
                tmp.path(),
                SearchQuery {
                    text: "content",
                    limit: 0,
                    where_clause: None,
                    mode,
                },
                NO_AUTO,
            )
            .await;
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
                SearchQuery {
                    text: "content",
                    limit: 10,
                    where_clause: Some("measurement_values IS NOT NULL"),
                    mode,
                },
                NO_AUTO,
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
            SearchQuery {
                text: "note",
                limit: 10,
                where_clause: Some("file_id = 'abc'"),
                mode: SearchMode::Semantic,
            },
            NO_AUTO,
        )
        .await;
        assert!(
            crate::step::has_failed(&result),
            "collision should fail the search"
        );
    }
}
