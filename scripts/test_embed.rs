#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! model2vec-rs = "0.1.4"
//! ```

use model2vec_rs::model::StaticModel;

// ============================================================================
// Embedder
// ============================================================================

enum Embedder {
    Model2Vec(StaticModel),
}

impl Embedder {
    /// Load a Model2Vec model from HuggingFace Hub (or local cache).
    fn model2vec(model_id: &str) -> Self {
        let model = StaticModel::from_pretrained(model_id, None, None, None)
            .unwrap_or_else(|e| panic!("failed to load model {model_id}: {e}"));
        Embedder::Model2Vec(model)
    }

    /// Embedding dimension.
    fn dimension(&self) -> usize {
        match self {
            Embedder::Model2Vec(model) => model.encode_single("probe").len(),
        }
    }

    /// Embed a single text.
    fn embed(&self, text: &str) -> Vec<f32> {
        match self {
            Embedder::Model2Vec(model) => model.encode_single(text),
        }
    }

    /// Embed a batch of texts.
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
// Tests
// ============================================================================

const TEST_MODEL: &str = "minishlab/potion-base-8M";

fn main() {
    println!("=== Embedding tests ===\n");

    // --- Test 1: Load model ---
    let embedder = Embedder::model2vec(TEST_MODEL);
    println!("  1. Model loaded: {}  ✓", TEST_MODEL);

    // --- Test 2: Dimension ---
    {
        let dim = embedder.dimension();
        assert!(dim > 0);
        println!("  2. Dimension: {}  ✓", dim);
    }

    // --- Test 3: Single embedding ---
    {
        let emb = embedder.embed("Hello world");
        assert_eq!(emb.len(), embedder.dimension());
        // Not all zeros
        assert!(emb.iter().any(|&v| v != 0.0));
        println!("  3. Single embedding: len={}  ✓", emb.len());
    }

    // --- Test 4: Batch embedding ---
    {
        let texts = &["First chunk", "Second chunk", "Third chunk"];
        let embeddings = embedder.embed_batch(texts);
        assert_eq!(embeddings.len(), 3);
        for emb in &embeddings {
            assert_eq!(emb.len(), embedder.dimension());
        }
        println!("  4. Batch embedding: {} vectors  ✓", embeddings.len());
    }

    // --- Test 5: Deterministic ---
    {
        let a = embedder.embed("same text");
        let b = embedder.embed("same text");
        assert_eq!(a, b);
        println!("  5. Deterministic  ✓");
    }

    // --- Test 6: Different texts produce different embeddings ---
    {
        let a = embedder.embed("cats are great");
        let b = embedder.embed("quantum computing theory");
        assert_ne!(a, b);
        println!("  6. Different texts → different embeddings  ✓");
    }

    // --- Test 7: Similar texts have higher cosine similarity ---
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
            "  7. Cosine: similar={:.4} > dissimilar={:.4}  ✓",
            sim_score, dis_score
        );
    }

    // --- Test 8: Empty string ---
    {
        let emb = embedder.embed("");
        assert_eq!(emb.len(), embedder.dimension());
        println!("  8. Empty string embedding: len={}  ✓", emb.len());
    }

    // --- Test 9: Batch matches individual ---
    {
        let texts = &["alpha", "beta"];
        let batch = embedder.embed_batch(texts);
        let individual_a = embedder.embed("alpha");
        let individual_b = embedder.embed("beta");
        assert_eq!(batch[0], individual_a);
        assert_eq!(batch[1], individual_b);
        println!("  9. Batch matches individual  ✓");
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
