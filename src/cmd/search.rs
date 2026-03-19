use crate::index::backend::Backend;
use crate::outcome::commands::SearchOutcome;
use crate::outcome::{
    EmbedQueryOutcome, ExecuteSearchOutcome, LoadModelOutcome, Outcome, ReadConfigOutcome,
    ReadIndexOutcome,
};
use crate::step::{from_pipeline_result, ErrorKind, Step, StepError, StepOutcome};
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
) -> Step<Outcome> {
    use crate::pipeline::embed::run_embed_query;
    use crate::pipeline::execute_search::run_execute_search;
    use crate::pipeline::load_model::run_load_model;
    use crate::pipeline::read_config::run_read_config;
    use crate::pipeline::read_index::run_read_index;

    let start = Instant::now();
    let mut substeps = Vec::new();

    // 1. Read config
    let (read_config_result, config) = run_read_config(path);
    substeps.push(from_pipeline_result(read_config_result, |o| {
        Outcome::ReadConfig(ReadConfigOutcome {
            config_path: o.config_path.clone(),
        })
    }));

    // Auto-build: run build before searching if configured
    let auto_build = if let Some(ref cfg) = config {
        let should_build = !no_build && cfg.search.as_ref().is_some_and(|s| s.auto_build);
        if should_build {
            let build_no_update = no_update || !cfg.search.as_ref().is_some_and(|s| s.auto_update);
            let build_step =
                crate::cmd::build::run(path, None, None, None, false, build_no_update, false).await;
            if build_step.has_failed_step() {
                substeps.push(build_step);
                return fail_msg(
                    &mut substeps,
                    start,
                    ErrorKind::User,
                    "auto-build failed",
                    4,
                );
            }
            Some(build_step)
        } else {
            None
        }
    } else {
        None
    };

    // Push auto-build substep if it ran
    if let Some(build_step) = auto_build {
        substeps.push(build_step);
    }

    // Re-read config if auto-build ran
    let config = if substeps.len() > 1 {
        // auto-build ran, re-read
        let (result, cfg) = run_read_config(path);
        substeps.push(from_pipeline_result(result, |o| {
            Outcome::ReadConfig(ReadConfigOutcome {
                config_path: o.config_path.clone(),
            })
        }));
        match cfg {
            Some(c) => Some(c),
            None => return fail_from_last(&mut substeps, start, 3),
        }
    } else {
        config
    };

    let embedding = config.as_ref().and_then(|c| c.embedding_model.as_ref());

    // 2. Read index
    let (read_index_result, index_data) = match &config {
        Some(_) => run_read_index(path),
        None => {
            substeps.push(Step {
                substeps: vec![],
                outcome: StepOutcome::Skipped,
            });
            return fail_from_last(&mut substeps, start, 3);
        }
    };
    substeps.push(from_pipeline_result(read_index_result, |o| {
        Outcome::ReadIndex(ReadIndexOutcome {
            exists: o.exists,
            files_indexed: o.files_indexed,
            chunks: o.chunks,
        })
    }));

    // Pre-checks before loading model
    let pre_check_error: Option<String> = match (config.as_ref(), embedding, index_data.as_ref()) {
        (None, _, _) => None, // already failed
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

    // 3. Load model
    if let Some(msg) = pre_check_error {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Complete {
                result: Err(StepError {
                    kind: ErrorKind::User,
                    message: msg,
                }),
                elapsed_ms: 0,
            },
        });
        return fail_from_last(&mut substeps, start, 2);
    }

    let emb_config = embedding.unwrap();
    let (load_model_result, embedder) = run_load_model(emb_config);
    substeps.push(from_pipeline_result(load_model_result, |o| {
        Outcome::LoadModel(LoadModelOutcome {
            model_name: o.model_name.clone(),
            dimension: o.dimension,
        })
    }));
    let embedder = match embedder {
        Some(e) => e,
        None => return fail_from_last(&mut substeps, start, 2),
    };

    // 4. Embed query
    let (embed_query_result, query_embedding) = run_embed_query(&embedder, query).await;
    substeps.push(from_pipeline_result(embed_query_result, |o| {
        Outcome::EmbedQuery(EmbedQueryOutcome {
            query: o.query.clone(),
        })
    }));
    let query_embedding = match query_embedding {
        Some(qe) => qe,
        None => return fail_from_last(&mut substeps, start, 1),
    };

    // 5. Execute search
    let cfg = config.as_ref().unwrap();
    let backend = Backend::parquet(path);
    let (prefix, aliases) = match &cfg.search {
        Some(sc) => (sc.internal_prefix.as_str(), &sc.aliases),
        None => ("", &std::collections::HashMap::new()),
    };
    let (execute_result, hits) = run_execute_search(
        &backend,
        query_embedding,
        where_clause,
        limit,
        prefix,
        aliases,
    )
    .await;
    substeps.push(from_pipeline_result(execute_result, |o| {
        Outcome::ExecuteSearch(ExecuteSearchOutcome { hits: o.hits })
    }));

    // Build result with chunk text populated (always — full outcome carries all data)
    let hits = match hits {
        Some(mut hits) => {
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
            hits
        }
        None => return fail_from_last(&mut substeps, start, 0),
    };

    let model_name = emb_config.name.clone();
    Step {
        substeps,
        outcome: StepOutcome::Complete {
            result: Ok(Outcome::Search(Box::new(SearchOutcome {
                query: query.to_string(),
                hits,
                model_name,
                limit,
            }))),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

fn fail_from_last(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    skipped: usize,
) -> Step<Outcome> {
    let msg = match substeps.iter().rev().find_map(|s| match &s.outcome {
        StepOutcome::Complete { result: Err(e), .. } => Some(e.message.clone()),
        _ => None,
    }) {
        Some(m) => m,
        None => "step failed".into(),
    };
    for _ in 0..skipped {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    }
    Step {
        substeps: std::mem::take(substeps),
        outcome: StepOutcome::Complete {
            result: Err(StepError {
                kind: ErrorKind::Application,
                message: msg,
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

fn fail_msg(
    substeps: &mut Vec<Step<Outcome>>,
    start: Instant,
    kind: ErrorKind,
    msg: &str,
    skipped: usize,
) -> Step<Outcome> {
    for _ in 0..skipped {
        substeps.push(Step {
            substeps: vec![],
            outcome: StepOutcome::Skipped,
        });
    }
    Step {
        substeps: std::mem::take(substeps),
        outcome: StepOutcome::Complete {
            result: Err(StepError {
                kind,
                message: msg.into(),
            }),
            elapsed_ms: start.elapsed().as_millis() as u64,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::{Embedder, ModelConfig};
    use crate::outcome::commands::SearchOutcome;
    use crate::schema::config::{FieldsConfig, MdvsToml, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, ScanConfig};
    use std::fs;

    fn unwrap_search(step: &Step<Outcome>) -> &SearchOutcome {
        match &step.outcome {
            StepOutcome::Complete {
                result: Ok(Outcome::Search(o)),
                ..
            } => o,
            other => panic!("expected Ok(Search), got: {other:?}"),
        }
    }

    fn unwrap_error(step: &Step<Outcome>) -> &StepError {
        match &step.outcome {
            StepOutcome::Complete { result: Err(e), .. } => e,
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
        let config = MdvsToml {
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
        assert!(!output.has_failed_step());
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(tmp.path(), "test query", 10, None, true, true, false).await;
        assert!(output.has_failed_step());
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let output = run(tmp.path(), "test query", 10, None, true, true, false).await;
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("index not found"));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "rust programming", 10, None, true, true, false).await;
        assert!(!output.has_failed_step(), "search failed: {:?}", output);
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
        assert!(!output.has_failed_step());
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

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
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
        assert!(output.has_failed_step());
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
        assert!(output.has_failed_step());
        let err = unwrap_error(&output);
        assert!(err.message.contains("unmatched single quote"));
    }

    #[tokio::test]
    async fn where_unmatched_double_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "test", 10, Some("x = \"bad"), true, true, false).await;
        assert!(output.has_failed_step());
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
        assert!(output.has_failed_step());
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
        if let StepOutcome::Complete { result: Err(e), .. } = &output.outcome {
            assert!(
                !e.message.contains("unmatched"),
                "balanced quotes should not trigger parity check"
            );
        }
    }
}
