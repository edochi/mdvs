use anyhow::Context;
use crate::index::backend::{IndexBackend, ParquetBackend};
use crate::index::embed::{Embedder, ModelConfig};
use crate::schema::config::MdvsToml;
use std::path::Path;

pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
) -> anyhow::Result<()> {
    let config_path = path.join("mdvs.toml");

    // Read config
    let config = MdvsToml::read(&config_path)?;
    let embedding = config.embedding_model.as_ref()
        .context("missing [embedding_model] in mdvs.toml (run `mdvs build` first)")?;

    let backend = ParquetBackend::new(path);

    // Index existence check (before loading model to fail fast)
    anyhow::ensure!(
        backend.exists(),
        "index not found (run `mdvs build` first)",
    );

    // Verify model matches index
    if let Some(ref meta) = backend.read_metadata()?
        && meta.embedding_model != *embedding
    {
        anyhow::bail!(
            "model mismatch: config has '{}' (rev {:?}) but index was built with '{}' (rev {:?}) — run 'mdvs build' to rebuild",
            embedding.name, embedding.revision,
            meta.embedding_model.name, meta.embedding_model.revision,
        );
    }

    // Load model
    eprintln!("Loading model {}...", embedding.name);
    let model_config = ModelConfig::Model2Vec {
        model_id: embedding.name.clone(),
        revision: embedding.revision.clone(),
    };
    let embedder = Embedder::load(&model_config);

    // Embed query
    let query_embedding = embedder.embed(query);

    // Search via backend
    let hits = backend.search(query_embedding, where_clause, limit).await?;

    // Print results
    for hit in &hits {
        println!("{:.3}  {}", hit.score, hit.filename);
    }

    Ok(())
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
                name: model_name.into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn init_and_build(dir: &Path) {
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
        init_and_build(tmp.path());

        let result = run(tmp.path(), "rust programming", 10, None).await;
        assert!(result.is_ok(), "search failed: {:?}", result);
    }

    #[tokio::test]
    async fn with_limit() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path());

        // Use backend to search with limit=1
        let backend = ParquetBackend::new(tmp.path());
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::Model2Vec {
            model_id: embedding.name.clone(),
            revision: embedding.revision.clone(),
        };
        let embedder = Embedder::load(&model_config);
        let query_embedding = embedder.embed("rust programming");

        let hits = backend.search(query_embedding, None, 1).await.unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[tokio::test]
    async fn with_where_clause() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path());

        let backend = ParquetBackend::new(tmp.path());
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let embedding = config.embedding_model.as_ref().unwrap();
        let model_config = ModelConfig::Model2Vec {
            model_id: embedding.name.clone(),
            revision: embedding.revision.clone(),
        };
        let embedder = Embedder::load(&model_config);
        let query_embedding = embedder.embed("cooking recipes");

        // Filter to non-draft only — cooking post (draft=true) should be excluded
        let hits = backend
            .search(query_embedding, Some("f.data['draft'] = false"), 10)
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
        init_and_build(tmp.path());

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
