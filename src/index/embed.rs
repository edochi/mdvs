use crate::schema::shared::EmbeddingModelConfig;
use model2vec_rs::model::StaticModel;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum ModelConfig {
    Model2Vec {
        model_id: String,
        revision: Option<String>,
    },
}

impl TryFrom<&EmbeddingModelConfig> for ModelConfig {
    type Error = anyhow::Error;

    fn try_from(config: &EmbeddingModelConfig) -> anyhow::Result<Self> {
        match config.provider.as_str() {
            "model2vec" => Ok(ModelConfig::Model2Vec {
                model_id: config.name.clone(),
                revision: config.revision.clone(),
            }),
            other => anyhow::bail!("unsupported embedding provider: '{other}'"),
        }
    }
}

pub enum Embedder {
    Model2Vec(StaticModel),
}

impl Embedder {
    pub fn load(config: &ModelConfig) -> Self {
        match config {
            ModelConfig::Model2Vec { model_id, revision } => {
                let model = StaticModel::from_pretrained(
                    model_id,
                    revision.as_deref(),
                    None,
                    None,
                )
                .unwrap_or_else(|e| panic!("failed to load model {model_id}: {e}"));
                Embedder::Model2Vec(model)
            }
        }
    }

    pub fn dimension(&self) -> usize {
        match self {
            Embedder::Model2Vec(model) => model.encode_single("probe").len(),
        }
    }

    pub async fn embed(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Model2Vec(model) => model.encode_single(text),
        }
    }

    pub async fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        match self {
            Embedder::Model2Vec(model) => {
                let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
                model.encode(&owned)
            }
        }
    }
}

pub fn resolve_revision(model_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let model_dir = model_id.replace('/', "--");
    let snapshots = PathBuf::from(home)
        .join(".cache/huggingface/hub")
        .join(format!("models--{model_dir}"))
        .join("snapshots");

    let entries = fs::read_dir(&snapshots).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_MODEL: &str = "minishlab/potion-base-8M";

    fn test_embedder() -> Embedder {
        let config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: None,
        };
        Embedder::load(&config)
    }

    #[test]
    fn load_from_config() {
        let _embedder = test_embedder();
    }

    #[test]
    fn resolve_revision_from_cache() {
        let _embedder = test_embedder();
        let rev = resolve_revision(TEST_MODEL);
        assert!(rev.is_some(), "model should be cached after loading");
        assert!(!rev.unwrap().is_empty());
    }

    #[tokio::test]
    async fn load_with_pinned_revision() {
        let embedder = test_embedder();
        let rev = resolve_revision(TEST_MODEL).unwrap();
        let pinned_config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: Some(rev),
        };
        let pinned_embedder = Embedder::load(&pinned_config);
        let a = embedder.embed("test").await;
        let b = pinned_embedder.embed("test").await;
        assert_eq!(a, b);
    }

    #[test]
    fn dimension() {
        let embedder = test_embedder();
        let dim = embedder.dimension();
        assert!(dim > 0);
    }

    #[tokio::test]
    async fn single_embedding() {
        let embedder = test_embedder();
        let emb = embedder.embed("Hello world").await;
        assert_eq!(emb.len(), embedder.dimension());
        assert!(emb.iter().any(|&v| v != 0.0));
    }

    #[tokio::test]
    async fn batch_embedding() {
        let embedder = test_embedder();
        let texts = &["First chunk", "Second chunk", "Third chunk"];
        let embeddings = embedder.embed_batch(texts).await;
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), embedder.dimension());
        }
    }

    #[tokio::test]
    async fn deterministic() {
        let embedder = test_embedder();
        let a = embedder.embed("same text").await;
        let b = embedder.embed("same text").await;
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn different_texts_different_embeddings() {
        let embedder = test_embedder();
        let a = embedder.embed("cats are great").await;
        let b = embedder.embed("quantum computing theory").await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn similar_texts_higher_cosine() {
        let embedder = test_embedder();
        let query = embedder.embed("rust programming language").await;
        let similar = embedder.embed("writing code in rust").await;
        let dissimilar = embedder.embed("chocolate cake recipe").await;

        let sim_score = cosine_similarity(&query, &similar);
        let dis_score = cosine_similarity(&query, &dissimilar);
        assert!(
            sim_score > dis_score,
            "similar={sim_score} should be > dissimilar={dis_score}"
        );
    }

    #[tokio::test]
    async fn empty_string_embedding() {
        let embedder = test_embedder();
        let emb = embedder.embed("").await;
        assert_eq!(emb.len(), embedder.dimension());
    }

    #[tokio::test]
    async fn batch_matches_individual() {
        let embedder = test_embedder();
        let texts = &["alpha", "beta"];
        let batch = embedder.embed_batch(texts).await;
        let individual_a = embedder.embed("alpha").await;
        let individual_b = embedder.embed("beta").await;
        assert_eq!(batch[0], individual_a);
        assert_eq!(batch[1], individual_b);
    }

    #[test]
    fn model_config_debug() {
        let rev = resolve_revision(TEST_MODEL).unwrap();
        let config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: Some(rev.clone()),
        };
        let debug = format!("{config:?}");
        assert!(debug.contains(TEST_MODEL));
        assert!(debug.contains(&rev));
    }
}
