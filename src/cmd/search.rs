use crate::index::backend::Backend;
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
use tracing::{instrument, warn};

/// Read lines from a file (1-indexed, inclusive range).
fn read_lines(path: &Path, start: i32, end: i32) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = (start - 1).max(0) as usize;
    let end = (end as usize).min(lines.len());
    if start >= end {
        return None;
    }
    Some(lines[start..end].join("\n"))
}

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
pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
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
            let backend = Backend::parquet(path);
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
                let build_meta = backend.read_metadata().ok().flatten();
                let idx_stats = backend.stats().ok().flatten();
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
                    emb.name, emb.revision,
                    data.metadata.embedding_model.name, data.metadata.embedding_model.revision,
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

    // 3. Load model (reuse from build if available)
    let emb_config = embedding.unwrap();
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

    // 5. Execute search — calls backend.search() directly with quote validation
    let cfg = config.as_ref().unwrap();
    let backend = Backend::parquet(path);
    let (prefix, aliases) = match &cfg.search {
        Some(sc) => (sc.internal_prefix.as_str(), &sc.aliases),
        None => ("", &std::collections::HashMap::new()),
    };

    if let Some(w) = where_clause {
        if let Err(msg) = validate_where_clause(w) {
            steps.push(StepEntry::err(ErrorKind::User, msg, 0));
            return CommandResult::failed_from_steps(std::mem::take(&mut steps), start);
        }
    }

    let search_start = Instant::now();
    let hits = match backend
        .search(query_embedding, where_clause, limit, prefix, aliases)
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

    // Populate chunk text for each hit
    let mut hits = hits;
    for hit in &mut hits {
        if let (Some(s), Some(e)) = (hit.start_line, hit.end_line) {
            match read_lines(&path.join(&hit.filename), s, e) {
                Some(text) => hit.chunk_text = Some(text),
                None => warn!(
                    file = %hit.filename,
                    "could not read chunk text (file may have changed since build)"
                ),
            }
        }
    }

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
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, ScanConfig};
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
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
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
        let step = crate::cmd::init::run(dir, "**", false, false, true, false, false);
        assert!(!crate::step::has_failed(&step));
        let output = crate::cmd::build::run(dir, None, None, None, false, true, false).await;
        assert!(!crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(tmp.path(), "test query", 10, None, true, true, false).await;
        assert!(crate::step::has_failed(&output));
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let output = run(tmp.path(), "test query", 10, None, true, true, false).await;
        assert!(crate::step::has_failed(&output));
        let err = unwrap_error(&output);
        assert!(err.message.contains("index not found"));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "rust programming", 10, None, true, true, false).await;
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

        let output = run(tmp.path(), "rust programming", 10, None, true, true, true).await;
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

        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::parquet(tmp.path());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("rust programming").await;

        let hits = backend
            .search(
                query_embedding,
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

        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::parquet(tmp.path());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("cooking recipes").await;

        let hits = backend
            .search(
                query_embedding,
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

        let output = run(tmp.path(), "test query", 10, None, true, true, false).await;
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

        let output = run(tmp.path(), "test", 10, Some("x = \"bad"), true, true, false).await;
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

    // --- Unit tests for read_lines ---

    #[test]
    fn read_lines_valid_range() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.md");
        std::fs::write(&file, "line1\nline2\nline3\nline4\n").unwrap();
        let result = read_lines(&file, 2, 3);
        assert_eq!(result, Some("line2\nline3".to_string()));
    }

    #[test]
    fn read_lines_out_of_bounds() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.md");
        std::fs::write(&file, "line1\n").unwrap();
        assert!(read_lines(&file, 10, 20).is_none());
    }

    #[test]
    fn read_lines_single_line() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("test.md");
        std::fs::write(&file, "only\n").unwrap();
        let result = read_lines(&file, 1, 1);
        assert_eq!(result, Some("only".to_string()));
    }
}
