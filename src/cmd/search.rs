use crate::index::embed::{Embedder, ModelConfig};
use crate::schema::config::MdvsToml;
use crate::search::SearchContext;
use datafusion::arrow::array::{Array, Float64Array, StringViewArray};
use std::path::Path;

pub async fn run(
    path: &Path,
    query: &str,
    limit: usize,
    where_clause: Option<&str>,
) -> anyhow::Result<()> {
    let config_path = path.join("mdvs.toml");
    let files_parquet = path.join(".mdvs/files.parquet");
    let chunks_parquet = path.join(".mdvs/chunks.parquet");

    // Read config
    let config = MdvsToml::read(&config_path)?;

    // Index existence check (before loading model to fail fast)
    anyhow::ensure!(
        files_parquet.exists(),
        "index not found: {} does not exist (run `mdvs build` first)",
        files_parquet.display(),
    );
    anyhow::ensure!(
        chunks_parquet.exists(),
        "index not found: {} does not exist (run `mdvs build` first)",
        chunks_parquet.display(),
    );

    // Load model
    eprintln!("Loading model {}...", config.model.name);
    let model_config = ModelConfig::Model2Vec {
        model_id: config.model.name.clone(),
        revision: config.model.revision.clone(),
    };
    let embedder = Embedder::load(&model_config);

    // Embed query
    let query_embedding = embedder.embed(query);

    // Create search context
    let sc = SearchContext::new(&files_parquet, &chunks_parquet, query_embedding).await?;

    // Build SQL
    let where_part = match where_clause {
        Some(w) => format!("WHERE {w}"),
        None => String::new(),
    };
    let sql = format!(
        "SELECT f.filename,
                MAX(cosine_similarity(c.embedding)) AS score
         FROM chunks c JOIN files f ON c.file_id = f.file_id
         {where_part}
         GROUP BY f.file_id, f.filename
         ORDER BY score DESC
         LIMIT {limit}"
    );

    // Execute query
    let batches = sc.query(&sql).await?;

    // Print results
    for batch in &batches {
        let filenames = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringViewArray>()
            .ok_or_else(|| anyhow::anyhow!("unexpected type for filename column"))?;
        let scores = batch
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| anyhow::anyhow!("unexpected type for score column"))?;

        for i in 0..batch.num_rows() {
            let filename = filenames.value(i);
            let score = scores.value(i);
            println!("{score:.3}  {filename}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{OnError, WorkflowConfig};
    use crate::schema::shared::{ChunkingConfig, ModelInfo, TomlConfig};
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
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: model_name.into(),
                revision: None,
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            workflow: WorkflowConfig {
                auto_build: true,
                on_error: OnError::Fail,
            },
            fields: vec![],
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn init_and_build(dir: &Path) {
        crate::cmd::init::run(
            dir,
            "minishlab/potion-base-8M",
            None,
            "**",
            false,
            false,
            true,
            1024,
            true,
        )
        .unwrap();
        crate::cmd::build::run(dir).unwrap();
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

        // Capture output by running the SQL directly
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let model_config = ModelConfig::Model2Vec {
            model_id: config.model.name.clone(),
            revision: config.model.revision.clone(),
        };
        let embedder = Embedder::load(&model_config);
        let query_embedding = embedder.embed("rust programming");

        let sc = SearchContext::new(
            &tmp.path().join(".mdvs/files.parquet"),
            &tmp.path().join(".mdvs/chunks.parquet"),
            query_embedding,
        )
        .await
        .unwrap();

        let sql = "SELECT f.filename,
                          MAX(cosine_similarity(c.embedding)) AS score
                   FROM chunks c JOIN files f ON c.file_id = f.file_id
                   GROUP BY f.file_id, f.filename
                   ORDER BY score DESC
                   LIMIT 1";

        let batches = sc.query(sql).await.unwrap();
        let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(total_rows, 1);
    }

    #[tokio::test]
    async fn with_where_clause() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path());

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        let model_config = ModelConfig::Model2Vec {
            model_id: config.model.name.clone(),
            revision: config.model.revision.clone(),
        };
        let embedder = Embedder::load(&model_config);
        let query_embedding = embedder.embed("cooking recipes");

        let sc = SearchContext::new(
            &tmp.path().join(".mdvs/files.parquet"),
            &tmp.path().join(".mdvs/chunks.parquet"),
            query_embedding,
        )
        .await
        .unwrap();

        // Filter to non-draft only — cooking post (draft=true) should be excluded
        let sql = "SELECT f.filename,
                          MAX(cosine_similarity(c.embedding)) AS score
                   FROM chunks c JOIN files f ON c.file_id = f.file_id
                   WHERE f.data['draft'] = false
                   GROUP BY f.file_id, f.filename
                   ORDER BY score DESC";

        let batches = sc.query(sql).await.unwrap();
        for batch in &batches {
            let filenames = batch
                .column(0)
                .as_any()
                .downcast_ref::<StringViewArray>()
                .unwrap();
            for i in 0..filenames.len() {
                assert_ne!(filenames.value(i), "blog/post2.md");
            }
        }
    }
}
