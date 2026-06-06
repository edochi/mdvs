use crate::schema::shared::EmbeddingModelConfig;
use model2vec_rs::model::StaticModel;
use std::fs;
use std::path::PathBuf;
use tracing::instrument;

/// Resolved embedding model configuration, ready for loading.
#[derive(Debug, Clone)]
pub enum ModelConfig {
    /// Model2Vec static model with optional pinned revision.
    Model2Vec {
        /// HuggingFace model identifier.
        model_id: String,
        /// Pinned commit SHA from the HuggingFace cache.
        revision: Option<String>,
    },
    /// Deterministic mock embedder — feature-gated, never selectable
    /// in production builds.
    #[cfg(any(test, feature = "testing-mocks"))]
    Mock {
        /// Embedding dimension. Defaults to 256 (matches potion-base-8M).
        dim: usize,
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
            #[cfg(any(test, feature = "testing-mocks"))]
            "mock" => Ok(ModelConfig::Mock {
                dim: config.dim.unwrap_or(256),
            }),
            #[cfg(not(any(test, feature = "testing-mocks")))]
            "mock" => anyhow::bail!(
                "embedding provider 'mock' is only available in builds compiled with \
                 `--features testing-mocks`"
            ),
            other => anyhow::bail!("unsupported embedding provider: '{other}'"),
        }
    }
}

/// Deterministic mock embedder. Vectors are derived by hashing the
/// input text and reinterpreting the hash-seeded byte stream as
/// little-endian `f32`s. Same input → same vector; distinct inputs
/// → distinct vectors. Not normalized (LanceDB handles cosine norm).
#[cfg(any(test, feature = "testing-mocks"))]
pub struct MockEmbedder {
    dim: usize,
}

#[cfg(any(test, feature = "testing-mocks"))]
impl MockEmbedder {
    fn new(dim: usize) -> Self {
        Self { dim }
    }

    fn encode(&self, text: &str) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.dim);
        let mut counter: u64 = 0;
        while out.len() < self.dim {
            let seed = xxhash_rust::xxh3::xxh3_64_with_seed(text.as_bytes(), counter);
            for shift in 0..2 {
                if out.len() >= self.dim {
                    break;
                }
                let word = (seed >> (shift * 32)) as u32;
                // Map u32 to [-0.5, 0.5] — always finite, centered around 0.
                out.push((word as f32) / (u32::MAX as f32) - 0.5);
            }
            counter += 1;
        }
        out
    }
}

/// Loaded embedding model, ready to produce vectors.
///
/// The size disparity between `Model2Vec` (~1.2 KB, owns a `StaticModel`)
/// and `Mock` (~8 bytes) is intentional — `Mock` only exists in test /
/// feature-gated builds and the indirection of `Box<StaticModel>` would
/// add a heap allocation to every embed call on the hot prod path.
#[allow(clippy::large_enum_variant)]
pub enum Embedder {
    /// A Model2Vec static model (CPU-only, no GPU required).
    Model2Vec(StaticModel),
    /// Deterministic mock embedder for hermetic tests.
    #[cfg(any(test, feature = "testing-mocks"))]
    Mock(MockEmbedder),
}

impl Embedder {
    /// Download (if needed) and load the model into memory.
    #[instrument(name = "load_model", skip_all, fields(model = ?config))]
    pub fn load(config: &ModelConfig) -> anyhow::Result<Self> {
        match config {
            ModelConfig::Model2Vec { model_id, revision } => {
                let model = StaticModel::from_pretrained(model_id, revision.as_deref(), None, None)
                    .map_err(|e| anyhow::anyhow!("failed to load model '{model_id}': {e}"))?;
                Ok(Embedder::Model2Vec(model))
            }
            #[cfg(any(test, feature = "testing-mocks"))]
            ModelConfig::Mock { dim } => Ok(Embedder::Mock(MockEmbedder::new(*dim))),
        }
    }

    /// Return the embedding dimension.
    pub fn dimension(&self) -> usize {
        match self {
            Embedder::Model2Vec(model) => model.encode_single("probe").len(),
            #[cfg(any(test, feature = "testing-mocks"))]
            Embedder::Mock(mock) => mock.dim,
        }
    }

    /// Embed a single text string into a dense vector.
    #[instrument(name = "embed", skip_all, level = "debug")]
    pub async fn embed(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Model2Vec(model) => model.encode_single(text),
            #[cfg(any(test, feature = "testing-mocks"))]
            Embedder::Mock(mock) => mock.encode(text),
        }
    }

    /// Embed multiple texts in a single batch call.
    #[instrument(name = "embed_batch", skip_all, fields(texts = texts.len()), level = "debug")]
    pub async fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        match self {
            Embedder::Model2Vec(model) => {
                let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
                model.encode(&owned)
            }
            #[cfg(any(test, feature = "testing-mocks"))]
            Embedder::Mock(mock) => texts.iter().map(|t| mock.encode(t)).collect(),
        }
    }
}

/// Look up the cached model revision (commit SHA) from the HuggingFace cache directory.
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

/// Compute the cosine similarity between two vectors, returning 0.0 for zero-norm inputs.
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

    /// Real-model tests. Marked `#[ignore]` so the default `cargo test`
    /// invocation skips them — they would otherwise download
    /// `minishlab/potion-base-8M` from Hugging Face. Run locally with
    /// `cargo test -- --ignored` once the model is cached.
    const TEST_MODEL: &str = "minishlab/potion-base-8M";

    fn test_embedder() -> Embedder {
        let config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: None,
        };
        Embedder::load(&config).expect("test model should load")
    }

    #[test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    fn load_from_config() {
        let _embedder = test_embedder();
    }

    #[test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    fn resolve_revision_from_cache() {
        let _embedder = test_embedder();
        let rev = resolve_revision(TEST_MODEL);
        assert!(rev.is_some(), "model should be cached after loading");
        assert!(!rev.unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    async fn load_with_pinned_revision() {
        let embedder = test_embedder();
        let rev = resolve_revision(TEST_MODEL).unwrap();
        let pinned_config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: Some(rev),
        };
        let pinned_embedder = Embedder::load(&pinned_config).expect("pinned model should load");
        let a = embedder.embed("test").await;
        let b = pinned_embedder.embed("test").await;
        assert_eq!(a, b);
    }

    #[test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    fn dimension() {
        let embedder = test_embedder();
        let dim = embedder.dimension();
        assert!(dim > 0);
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    async fn single_embedding() {
        let embedder = test_embedder();
        let emb = embedder.embed("Hello world").await;
        assert_eq!(emb.len(), embedder.dimension());
        assert!(emb.iter().any(|&v| v != 0.0));
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
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
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    async fn deterministic() {
        let embedder = test_embedder();
        let a = embedder.embed("same text").await;
        let b = embedder.embed("same text").await;
        assert_eq!(a, b);
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    async fn different_texts_different_embeddings() {
        let embedder = test_embedder();
        let a = embedder.embed("cats are great").await;
        let b = embedder.embed("quantum computing theory").await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
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
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
    async fn empty_string_embedding() {
        let embedder = test_embedder();
        let emb = embedder.embed("").await;
        assert_eq!(emb.len(), embedder.dimension());
    }

    #[tokio::test]
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
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
    #[ignore = "loads real HF model — slow lane, run with `cargo test -- --ignored`"]
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

#[cfg(test)]
mod mock_tests {
    use super::*;

    fn mock_embedder(dim: usize) -> Embedder {
        Embedder::load(&ModelConfig::Mock { dim }).unwrap()
    }

    #[test]
    fn mock_dimension_matches_config() {
        let embedder = mock_embedder(128);
        assert_eq!(embedder.dimension(), 128);
    }

    #[tokio::test]
    async fn mock_embed_returns_correct_length() {
        let embedder = mock_embedder(256);
        let emb = embedder.embed("hello").await;
        assert_eq!(emb.len(), 256);
        assert!(emb.iter().all(|v| v.is_finite()));
    }

    #[tokio::test]
    async fn mock_is_deterministic() {
        let embedder = mock_embedder(64);
        let a = embedder.embed("same text").await;
        let b = embedder.embed("same text").await;
        assert_eq!(a, b);
    }

    #[tokio::test]
    async fn mock_distinct_inputs_distinct_vectors() {
        let embedder = mock_embedder(64);
        let a = embedder.embed("alpha").await;
        let b = embedder.embed("beta").await;
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn mock_batch_matches_individual() {
        let embedder = mock_embedder(64);
        let batch = embedder.embed_batch(&["foo", "bar"]).await;
        let solo_foo = embedder.embed("foo").await;
        let solo_bar = embedder.embed("bar").await;
        assert_eq!(batch[0], solo_foo);
        assert_eq!(batch[1], solo_bar);
    }

    #[test]
    fn mock_provider_via_try_from() {
        let cfg = crate::schema::shared::EmbeddingModelConfig {
            provider: "mock".into(),
            name: "mock".into(),
            revision: None,
            dim: Some(128),
        };
        let model_config = ModelConfig::try_from(&cfg).unwrap();
        match model_config {
            ModelConfig::Mock { dim } => assert_eq!(dim, 128),
            other => panic!("expected Mock, got {other:?}"),
        }
    }

    #[test]
    fn mock_provider_dim_defaults_to_256() {
        let cfg = crate::schema::shared::EmbeddingModelConfig {
            provider: "mock".into(),
            name: "mock".into(),
            revision: None,
            dim: None,
        };
        let model_config = ModelConfig::try_from(&cfg).unwrap();
        match model_config {
            ModelConfig::Mock { dim } => assert_eq!(dim, 256),
            other => panic!("expected Mock, got {other:?}"),
        }
    }
}
