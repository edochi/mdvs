use crate::index::backend::{Backend, SearchHit};
use crate::index::embed::{Embedder, ModelConfig};
use crate::output::CommandOutput;
use crate::schema::config::MdvsToml;
use crate::table::{style_compact, style_record, Builder};
use anyhow::Context;
use serde::Serialize;
use std::path::Path;
use tracing::{info, instrument, warn};

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
    /// Wall-clock time for the search operation in milliseconds.
    pub elapsed_ms: u64,
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
                "{} {} | model: \"{}\" | limit: {} | {}ms\n",
                self.hits.len(),
                hit_word,
                self.model_name,
                self.limit,
                self.elapsed_ms
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

/// Embed a query, search the index, and return ranked results.
#[instrument(name = "search", skip_all)]
pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
    verbose: bool,
) -> anyhow::Result<SearchResult> {
    // Validate --where clause: unmatched quotes indicate unescaped special characters
    if let Some(w) = where_clause {
        if w.chars().filter(|&c| c == '\'').count() % 2 != 0 {
            anyhow::bail!(
                "unmatched single quote in --where clause — escape with '' (e.g. O''Brien)"
            );
        }
        if w.chars().filter(|&c| c == '"').count() % 2 != 0 {
            anyhow::bail!(
                "unmatched double quote in --where clause — escape with \"\" (e.g. \"\"field\"\")"
            );
        }
    }

    let start = std::time::Instant::now();
    let config_path = path.join("mdvs.toml");

    // Read config
    let config = MdvsToml::read(&config_path)?;
    let embedding = config
        .embedding_model
        .as_ref()
        .context("missing [embedding_model] in mdvs.toml (run `mdvs build` first)")?;

    let backend = Backend::parquet(path, config.internal_prefix());

    // Index existence check (before loading model to fail fast)
    anyhow::ensure!(backend.exists(), "index not found (run `mdvs build` first)",);

    // Verify model matches index
    if let Some(ref meta) = backend.read_metadata()? {
        if meta.embedding_model != *embedding {
            anyhow::bail!(
                "model mismatch: config has '{}' (rev {:?}) but index was built with '{}' (rev {:?}) — run 'mdvs build' to rebuild",
                embedding.name, embedding.revision,
                meta.embedding_model.name, meta.embedding_model.revision,
            );
        }
    }

    // Load model
    info!(model = %embedding.name, "loading model");
    let t = std::time::Instant::now();
    let model_config = ModelConfig::try_from(embedding)?;
    let embedder = Embedder::load(&model_config)?;
    info!(elapsed_ms = t.elapsed().as_millis() as u64, "model loaded");

    // Embed query
    let query_embedding = embedder.embed(query).await;

    // Search via backend
    let t = std::time::Instant::now();
    let mut hits = backend.search(query_embedding, where_clause, limit).await?;
    info!(
        hits = hits.len(),
        elapsed_ms = t.elapsed().as_millis() as u64,
        "search complete"
    );

    // Populate chunk text from disk when verbose
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

    let model_name = embedding.name.clone();

    Ok(SearchResult {
        query: query.to_string(),
        hits,
        model_name,
        limit,
        elapsed_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, SearchConfig, UpdateConfig};
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
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    async fn init_and_build(dir: &Path) {
        crate::cmd::init::run(
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
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run(tmp.path(), "test query", 10, None, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let result = run(tmp.path(), "test query", 10, None, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("index not found"));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let result = run(tmp.path(), "rust programming", 10, None, false).await;
        assert!(result.is_ok(), "search failed: {:?}", result);
        let result = result.unwrap();
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

        let result = run(tmp.path(), "rust programming", 10, None, true)
            .await
            .unwrap();
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
        let backend = Backend::parquet(tmp.path(), config.internal_prefix());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("rust programming").await;

        let hits = backend.search(query_embedding, None, 1).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn with_where_clause() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::parquet(tmp.path(), config.internal_prefix());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config).unwrap();
        let query_embedding = embedder.embed("cooking recipes").await;

        let hits = backend
            .search(query_embedding, Some("draft = false"), 10)
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

        let result = run(tmp.path(), "test query", 10, None, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("model mismatch"));
    }

    #[tokio::test]
    async fn where_unmatched_single_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let result = run(tmp.path(), "test", 10, Some("author = 'O'Brien'"), false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unmatched single quote"));
        assert!(err.contains("O''Brien"));
    }

    #[tokio::test]
    async fn where_unmatched_double_quote() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let result = run(tmp.path(), "test", 10, Some("x = \"bad"), false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("unmatched double quote"));
    }

    #[tokio::test]
    async fn where_even_but_malformed_quotes() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // 2 single quotes (even) — passes parity check but DataFusion rejects it
        let result = run(
            tmp.path(),
            "test",
            10,
            Some("author's name = O'Brien"),
            false,
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn where_balanced_quotes_pass() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // Properly escaped single quotes should pass validation
        let result = run(tmp.path(), "test", 10, Some("title = 'O''Brien'"), false).await;
        // Should not fail with quote parity error (may fail for other reasons like no match)
        if let Err(e) = &result {
            assert!(
                !e.to_string().contains("unmatched"),
                "balanced quotes should not trigger parity check"
            );
        }
    }
}
