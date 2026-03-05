use anyhow::Context;
use crate::index::backend::{Backend, SearchHit};
use crate::index::embed::{Embedder, ModelConfig};
use crate::output::CommandOutput;
use crate::schema::config::MdvsToml;
use serde::Serialize;
use std::path::Path;
use tracing::{info, instrument};

/// Result of the `search` command: ranked list of matching files.
#[derive(Debug, Serialize)]
pub struct SearchResult {
    /// Files ranked by cosine similarity to the query, descending.
    pub hits: Vec<SearchHit>,
}

impl CommandOutput for SearchResult {
    fn format_text(&self, _verbose: bool) -> String {
        self.hits
            .iter()
            .map(|h| format!("{:.3}  {}\n", h.score, h.filename))
            .collect()
    }
}

/// Embed a query, search the index, and return ranked results.
#[instrument(name = "search", skip_all)]
pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
) -> anyhow::Result<SearchResult> {
    let config_path = path.join("mdvs.toml");

    // Read config
    let config = MdvsToml::read(&config_path)?;
    let embedding = config.embedding_model.as_ref()
        .context("missing [embedding_model] in mdvs.toml (run `mdvs build` first)")?;

    let backend = Backend::parquet(path, config.internal_prefix());

    // Index existence check (before loading model to fail fast)
    anyhow::ensure!(
        backend.exists(),
        "index not found (run `mdvs build` first)",
    );

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
    let embedder = Embedder::load(&model_config);
    info!(elapsed_ms = t.elapsed().as_millis() as u64, "model loaded");

    // Embed query
    let query_embedding = embedder.embed(query).await;

    // Search via backend
    let t = std::time::Instant::now();
    let hits = backend.search(query_embedding, where_clause, limit).await?;
    info!(hits = hits.len(), elapsed_ms = t.elapsed().as_millis() as u64, "search complete");

    Ok(SearchResult { hits })
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
            true, // auto_build calls build internally
            false, // skip_gitignore
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run(tmp.path(), "test query", 10, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn missing_index() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model");

        let result = run(tmp.path(), "test query", 10, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("index not found"));
    }

    #[tokio::test]
    async fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        let result = run(tmp.path(), "rust programming", 10, None).await;
        assert!(result.is_ok(), "search failed: {:?}", result);
    }

    #[tokio::test]
    async fn with_limit() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path()).await;

        // Use backend to search with limit=1
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let backend = Backend::parquet(tmp.path(), config.internal_prefix());
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::try_from(embedding).unwrap();
        let embedder = Embedder::load(&model_config);
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
        let embedder = Embedder::load(&model_config);
        let query_embedding = embedder.embed("cooking recipes").await;

        // Filter to non-draft only — cooking post (draft=true) should be excluded
        // Uses bare field name (promoted via files_v view)
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

        // Overwrite config with a different model name
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.embedding_model.as_mut().unwrap().name = "some-other-model".into();
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), "test query", 10, None).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("model mismatch"));
    }
}
