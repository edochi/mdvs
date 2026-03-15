use crate::index::backend::{Backend, SearchHit};
use crate::output::{format_json_compact, CommandOutput};
use crate::pipeline::embed::EmbedQueryOutput;
use crate::pipeline::execute_search::ExecuteSearchOutput;
use crate::pipeline::load_model::LoadModelOutput;
use crate::pipeline::read_config::ReadConfigOutput;
use crate::pipeline::read_index::ReadIndexOutput;
use crate::pipeline::{ErrorKind, ProcessingStepError, ProcessingStepResult};
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::path::Path;
use tracing::{instrument, warn};

/// Result of the `search` command: ranked list of matching files.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    /// The query string.
    pub query: String,
    /// Files ranked by cosine similarity to the query, descending.
    pub hits: Vec<SearchHit>,
    /// Name of the embedding model used.
    pub model_name: String,
    /// Result limit that was applied.
    pub limit: usize,
}

impl CommandOutput for SearchResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // One-liner
        let hit_word = if self.hits.len() == 1 { "hit" } else { "hits" };
        out.push_str(&format!(
            "Searched \"{}\" — {} {}\n",
            self.query,
            self.hits.len(),
            hit_word
        ));

        if self.hits.is_empty() {
            return out;
        }

        out.push('\n');

        if verbose {
            // Record tables: one per hit with chunk text
            for (i, hit) in self.hits.iter().enumerate() {
                let mut builder = Builder::default();
                let idx = format!("{}", i + 1);
                let path = format!("\"{}\"", hit.filename);
                let score = format!("{:.3}", hit.score);
                builder.push_record([idx.as_str(), path.as_str(), score.as_str()]);

                let detail = match (&hit.chunk_text, hit.start_line, hit.end_line) {
                    (Some(text), Some(start), Some(end)) => {
                        let indented: String = text
                            .lines()
                            .map(|l| format!("    {l}"))
                            .collect::<Vec<_>>()
                            .join("\n");
                        format!("  lines {start}-{end}:\n{indented}")
                    }
                    (None, Some(start), Some(end)) => format!("  lines {start}-{end}"),
                    _ => String::new(),
                };

                builder.push_record([detail.as_str(), "", ""]);
                let mut table = builder.build();
                style_record(&mut table, 3);
                out.push_str(&format!("{table}\n"));
            }

            // Footer
            out.push_str(&format!(
                "{} {} | model: \"{}\" | limit: {}\n",
                self.hits.len(),
                hit_word,
                self.model_name,
                self.limit,
            ));
        } else {
            // Compact table
            let mut builder = Builder::default();
            for (i, hit) in self.hits.iter().enumerate() {
                let idx = format!("{}", i + 1);
                let path = format!("\"{}\"", hit.filename);
                let score = format!("{:.3}", hit.score);
                builder.push_record([idx.as_str(), path.as_str(), score.as_str()]);
            }
            let mut table = builder.build();
            style_compact(&mut table);
            out.push_str(&format!("{table}\n"));
        }

        out
    }
}

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

// ============================================================================
// SearchCommandOutput (pipeline-based)
// ============================================================================

/// Pipeline record for the search command.
#[derive(Debug, Serialize)]
pub struct SearchProcessOutput {
    /// Read config step result.
    pub read_config: ProcessingStepResult<ReadConfigOutput>,
    /// Read index step result.
    pub read_index: ProcessingStepResult<ReadIndexOutput>,
    /// Load model step result.
    pub load_model: ProcessingStepResult<LoadModelOutput>,
    /// Embed query step result.
    pub embed_query: ProcessingStepResult<EmbedQueryOutput>,
    /// Execute search step result.
    pub execute_search: ProcessingStepResult<ExecuteSearchOutput>,
}

/// Full output of the search command: pipeline steps + command result.
#[derive(Debug, Serialize)]
pub struct SearchCommandOutput {
    /// Processing steps and their outcomes.
    pub process: SearchProcessOutput,
    /// Command result (None if pipeline didn't complete).
    pub result: Option<SearchResult>,
}

impl SearchCommandOutput {
    /// Returns `true` if any processing step failed.
    pub fn has_failed_step(&self) -> bool {
        matches!(self.process.read_config, ProcessingStepResult::Failed(_))
            || matches!(self.process.read_index, ProcessingStepResult::Failed(_))
            || matches!(self.process.load_model, ProcessingStepResult::Failed(_))
            || matches!(self.process.embed_query, ProcessingStepResult::Failed(_))
            || matches!(self.process.execute_search, ProcessingStepResult::Failed(_))
    }
}

impl CommandOutput for SearchCommandOutput {
    fn format_json(&self, verbose: bool) -> String {
        format_json_compact(self, self.result.as_ref(), verbose)
    }

    fn format_text(&self, verbose: bool) -> String {
        if let Some(result) = &self.result {
            if verbose {
                let mut out = String::new();
                out.push_str(&format!(
                    "{}\n",
                    self.process.read_config.format_line("Read config")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.read_index.format_line("Read index")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.load_model.format_line("Load model")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.embed_query.format_line("Embed query")
                ));
                out.push_str(&format!(
                    "{}\n",
                    self.process.execute_search.format_line("Search")
                ));
                out.push('\n');
                out.push_str(&result.format_text(verbose));
                out
            } else {
                result.format_text(verbose)
            }
        } else {
            // Pipeline didn't complete — show steps up to the failure
            let mut out = String::new();
            out.push_str(&format!(
                "{}\n",
                self.process.read_config.format_line("Read config")
            ));
            if !matches!(self.process.read_index, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.read_index.format_line("Read index")
                ));
            }
            if !matches!(self.process.load_model, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.load_model.format_line("Load model")
                ));
            }
            if !matches!(self.process.embed_query, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.embed_query.format_line("Embed query")
                ));
            }
            if !matches!(self.process.execute_search, ProcessingStepResult::Skipped) {
                out.push_str(&format!(
                    "{}\n",
                    self.process.execute_search.format_line("Search")
                ));
            }
            out
        }
    }
}

/// Embed a query, search the index, and return ranked results.
#[instrument(name = "search", skip_all)]
pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
    verbose: bool,
) -> SearchCommandOutput {
    use crate::pipeline::embed::run_embed_query;
    use crate::pipeline::execute_search::run_execute_search;
    use crate::pipeline::load_model::run_load_model;
    use crate::pipeline::read_config::run_read_config;
    use crate::pipeline::read_index::run_read_index;

    let (read_config_step, config) = run_read_config(path);

    let embedding = config.as_ref().and_then(|c| c.embedding_model.as_ref());

    let (read_index_step, index_data) = match &config {
        Some(_) => run_read_index(path),
        None => (ProcessingStepResult::Skipped, None),
    };

    // Pre-checks before loading model (fail fast on user errors)
    let pre_check_error: Option<String> = if config.is_none() {
        None // already failed at read_config
    } else if embedding.is_none() {
        Some("missing [embedding_model] in mdvs.toml (run `mdvs build` first)".to_string())
    } else if matches!(read_index_step, ProcessingStepResult::Failed(_)) {
        None // already failed at read_index
    } else if index_data.is_none() {
        Some("index not found (run `mdvs build` first)".to_string())
    } else {
        // Model mismatch check
        let data = index_data.as_ref().unwrap();
        let emb = embedding.unwrap();
        if data.metadata.embedding_model != *emb {
            Some(format!(
                "model mismatch: config has '{}' (rev {:?}) but index was built with '{}' (rev {:?}) — run 'mdvs build' to rebuild",
                emb.name, emb.revision,
                data.metadata.embedding_model.name, data.metadata.embedding_model.revision,
            ))
        } else {
            None
        }
    };

    let (load_model_step, embedder) = match (embedding, &pre_check_error) {
        (Some(emb), None) => run_load_model(emb),
        (_, Some(msg)) => {
            let err = ProcessingStepError {
                kind: ErrorKind::User,
                message: msg.clone(),
            };
            (ProcessingStepResult::Failed(err), None)
        }
        _ => (ProcessingStepResult::Skipped, None),
    };

    let (embed_query_step, query_embedding) = match &embedder {
        Some(emb) => run_embed_query(emb, query).await,
        None => (ProcessingStepResult::Skipped, None),
    };

    let (execute_search_step, hits) = match (&config, query_embedding) {
        (Some(cfg), Some(qe)) => {
            let backend = Backend::parquet(path);
            let (prefix, aliases) = match &cfg.search {
                Some(sc) => (sc.internal_prefix.as_str(), &sc.aliases),
                None => ("", &std::collections::HashMap::new()),
            };
            run_execute_search(&backend, qe, where_clause, limit, prefix, aliases).await
        }
        _ => (ProcessingStepResult::Skipped, None),
    };

    // Build result with chunk text populated if verbose
    let result = hits.map(|mut hits| {
        if verbose {
            for hit in &mut hits {
                if let (Some(start), Some(end)) = (hit.start_line, hit.end_line) {
                    match read_lines(&path.join(&hit.filename), start, end) {
                        Some(text) => hit.chunk_text = Some(text),
                        None => warn!(
                            file = %hit.filename,
                            "could not read chunk text (file may have changed since build)"
                        ),
                    }
                }
            }
        }
        let model_name = embedding.map(|e| e.name.clone()).unwrap_or_default();
        SearchResult {
            query: query.to_string(),
            hits,
            model_name,
            limit,
        }
    });

    SearchCommandOutput {
        process: SearchProcessOutput {
            read_config: read_config_step,
            read_index: read_index_step,
            load_model: load_model_step,
            embed_query: embed_query_step,
            execute_search: execute_search_step,
        },
        result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::{Embedder, ModelConfig};
    use crate::schema::config::{FieldsConfig, MdvsToml, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, ScanConfig};
    use std::fs;

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
            update: UpdateConfig { auto_build: true },
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
            search: Some(SearchConfig {
                default_limit: 10,
                internal_prefix: String::new(),
                aliases: std::collections::HashMap::new(),
            }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    async fn init_and_build(dir: &Path) {
        let output = crate::cmd::init::run(
            dir,
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
            false, // verbose
        )
        .await;
        assert!(!output.has_failed_step());
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let output = run(tmp.path(), "test query", 10, None, false).await;
        assert!(output.has_failed_step());
        assert!(output.result.is_none());
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let output = run(tmp.path(), "test query", 10, None, false).await;
        assert!(output.has_failed_step());
        assert!(output.result.is_none());
        if let ProcessingStepResult::Failed(err) = &output.process.load_model {
            assert!(err.message.contains("index not found"));
        } else {
            panic!("expected load_model to fail");
        }
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "rust programming", 10, None, false).await;
        assert!(!output.has_failed_step(), "search failed: {:?}", output);
        let result = output.result.unwrap();
        assert_eq!(result.query, "rust programming");
        assert!(!result.model_name.is_empty());
        assert!(!result.hits.is_empty());
        // start_line/end_line always present
        assert!(result.hits[0].start_line.is_some());
        assert!(result.hits[0].end_line.is_some());
        // chunk_text not populated without verbose
        assert!(result.hits[0].chunk_text.is_none());
    }

    #[tokio::test]
    async fn end_to_end_verbose() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "rust programming", 10, None, true).await;
        assert!(!output.has_failed_step());
        let result = output.result.unwrap();
        assert!(!result.hits.is_empty());
        // chunk_text populated in verbose mode
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

        let output = run(tmp.path(), "test query", 10, None, false).await;
        assert!(output.has_failed_step());
        if let ProcessingStepResult::Failed(err) = &output.process.load_model {
            assert!(err.message.contains("model mismatch"));
        } else {
            panic!("expected load_model to fail");
        }
    }

    #[tokio::test]
    async fn where_unmatched_single_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "test", 10, Some("author = 'O'Brien'"), false).await;
        assert!(output.has_failed_step());
        if let ProcessingStepResult::Failed(err) = &output.process.execute_search {
            assert!(err.message.contains("unmatched single quote"));
            assert!(err.message.contains("O''Brien"));
        } else {
            panic!("expected execute_search to fail");
        }
    }

    #[tokio::test]
    async fn where_unmatched_double_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let output = run(tmp.path(), "test", 10, Some("x = \"bad"), false).await;
        assert!(output.has_failed_step());
        if let ProcessingStepResult::Failed(err) = &output.process.execute_search {
            assert!(err.message.contains("unmatched double quote"));
        } else {
            panic!("expected execute_search to fail");
        }
    }

    #[tokio::test]
    async fn where_even_but_malformed_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // 2 single quotes (even) — passes parity check but DataFusion rejects it
        let output = run(
            tmp.path(),
            "test",
            10,
            Some("author's name = O'Brien"),
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

        // Properly escaped single quotes should pass validation
        let output = run(tmp.path(), "test", 10, Some("title = 'O''Brien'"), false).await;
        // Should not fail with quote parity error (may fail for other reasons like no match)
        if let ProcessingStepResult::Failed(err) = &output.process.execute_search {
            assert!(
                !err.message.contains("unmatched"),
                "balanced quotes should not trigger parity check"
            );
        }
    }
}
