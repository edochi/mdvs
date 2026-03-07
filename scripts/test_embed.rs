#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! model2vec-rs = "0.1.4"
//! ```

use model2vec_rs::model::StaticModel;

// ============================================================================
// ModelConfig — serializable, lives in mdvs.toml / mdvs.lock
// ============================================================================

#[derive(Debug, Clone)]
enum ModelConfig {
    Model2Vec {
        model_id: String,
        revision: Option<String>,
    },
}

// ============================================================================
// Embedder — runtime, loaded model
// ============================================================================

enum Embedder {
    Model2Vec(StaticModel),
}

impl Embedder {
    fn load(config: &ModelConfig) -> Self {
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

    fn dimension(&self) -> usize {
        match self {
            Embedder::Model2Vec(model) => model.encode_single("probe").len(),
        }
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Model2Vec(model) => model.encode_single(text),
        }
    }

    fn embed_batch(&self, texts: &[&str]) -> Vec<Vec<f32>> {
        match self {
            Embedder::Model2Vec(model) => {
                let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
                model.encode(&owned)
            }
        }
    }
}

// ============================================================================
// Resolve revision from HuggingFace cache
// ============================================================================

fn resolve_revision(model_id: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let model_dir = model_id.replace('/', "--");
    let snapshots = std::path::PathBuf::from(home)
        .join(".cache/huggingface/hub")
        .join(format!("models--{model_dir}"))
        .join("snapshots");

    let entries = std::fs::read_dir(&snapshots).ok()?;
    for entry in entries {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

const TEST_MODEL: &str = "minishlab/potion-base-8M";

fn main() {
    println!("=== Embedding tests ===\n");

    // --- Test 1: Load from ModelConfig (no revision) ---
    let config = ModelConfig::Model2Vec {
        model_id: TEST_MODEL.into(),
        revision: None,
    };
    let embedder = Embedder::load(&config).expect("failed to load model");
    println!("  1. Loaded from ModelConfig (no revision)  ✓");

    // --- Test 2: Resolve revision from cache ---
    {
        let rev = resolve_revision(TEST_MODEL);
        assert!(rev.is_some(), "model should be cached after loading");
        let rev = rev.unwrap();
        assert!(!rev.is_empty());
        println!("  2. Resolved revision: {}  ✓", rev);
    }

    // --- Test 3: Load with pinned revision ---
    {
        let rev = resolve_revision(TEST_MODEL).unwrap();
        let pinned_config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: Some(rev.clone()),
        };
        let pinned_embedder = Embedder::load(&pinned_config).expect("failed to load pinned model");
        // Should produce same embeddings as unpinned
        let a = embedder.embed("test");
        let b = pinned_embedder.embed("test");
        assert_eq!(a, b);
        println!("  3. Pinned revision matches unpinned  ✓");
    }

    // --- Test 4: Dimension ---
    {
        let dim = embedder.dimension();
        assert!(dim > 0);
        println!("  4. Dimension: {}  ✓", dim);
    }

    // --- Test 5: Single embedding ---
    {
        let emb = embedder.embed("Hello world");
        assert_eq!(emb.len(), embedder.dimension());
        assert!(emb.iter().any(|&v| v != 0.0));
        println!("  5. Single embedding: len={}  ✓", emb.len());
    }

    // --- Test 6: Batch embedding ---
    {
        let texts = &["First chunk", "Second chunk", "Third chunk"];
        let embeddings = embedder.embed_batch(texts);
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), embedder.dimension());
        }
        println!("  6. Batch embedding: {} vectors  ✓", embeddings.len());
    }

    // --- Test 7: Deterministic ---
    {
        let a = embedder.embed("same text");
        let b = embedder.embed("same text");
        assert_eq!(a, b);
        println!("  7. Deterministic  ✓");
    }

    // --- Test 8: Different texts → different embeddings ---
    {
        let a = embedder.embed("cats are great");
        let b = embedder.embed("quantum computing theory");
        assert_ne!(a, b);
        println!("  8. Different texts → different embeddings  ✓");
    }

    // --- Test 9: Similar texts have higher cosine similarity ---
    {
        let query = embedder.embed("rust programming language");
        let similar = embedder.embed("writing code in rust");
        let dissimilar = embedder.embed("chocolate cake recipe");

        let sim_score = cosine_similarity(&query, &similar);
        let dis_score = cosine_similarity(&query, &dissimilar);
        assert!(
            sim_score > dis_score,
            "similar={sim_score} should be > dissimilar={dis_score}"
        );
        println!(
            "  9. Cosine: similar={:.4} > dissimilar={:.4}  ✓",
            sim_score, dis_score
        );
    }

    // --- Test 10: Empty string ---
    {
        let emb = embedder.embed("");
        assert_eq!(emb.len(), embedder.dimension());
        println!("  10. Empty string embedding: len={}  ✓", emb.len());
    }

    // --- Test 11: Batch matches individual ---
    {
        let texts = &["alpha", "beta"];
        let batch = embedder.embed_batch(texts);
        let individual_a = embedder.embed("alpha");
        let individual_b = embedder.embed("beta");
        assert_eq!(batch[0], individual_a);
        assert_eq!(batch[1], individual_b);
        println!("  11. Batch matches individual  ✓");
    }

    // --- Test 12: ModelConfig Debug ---
    {
        let rev = resolve_revision(TEST_MODEL).unwrap();
        let full_config = ModelConfig::Model2Vec {
            model_id: TEST_MODEL.into(),
            revision: Some(rev.clone()),
        };
        let debug = format!("{:?}", full_config);
        assert!(debug.contains(TEST_MODEL));
        assert!(debug.contains(&rev));
        println!("  12. ModelConfig Debug: {}  ✓", debug);
    }

    println!("\n=== All tests passed ===");
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}
