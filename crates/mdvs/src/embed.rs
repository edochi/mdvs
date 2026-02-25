use anyhow::{Context, Result};
use model2vec_rs::model::StaticModel;

/// Load a model from the HuggingFace Hub (or local cache).
pub fn load_model(model_id: &str) -> Result<StaticModel> {
    StaticModel::from_pretrained(model_id, None, None, None)
        .with_context(|| format!("Failed to load model: {model_id}"))
}

/// Probe the model's embedding dimension by encoding a dummy string.
pub fn get_dimension(model: &StaticModel) -> usize {
    let probe = model.encode_single("dimension probe");
    probe.len()
}

/// Encode a batch of texts into embeddings.
pub fn encode_batch(model: &StaticModel, texts: &[String]) -> Vec<Vec<f32>> {
    model.encode(texts)
}

/// Encode a single query string.
pub fn encode_query(model: &StaticModel, query: &str) -> Vec<f32> {
    model.encode_single(query)
}

/// Resolve the cached model revision by reading the HuggingFace cache directory.
pub fn resolve_revision(model_id: &str) -> Option<String> {
    let cache_dir = dirs_cache_path(model_id)?;
    let snapshots = cache_dir.join("snapshots");
    let entries = std::fs::read_dir(&snapshots).ok()?;

    // Return the first (usually only) snapshot hash
    for entry in entries {
        let entry = entry.ok()?;
        if entry.file_type().ok()?.is_dir() {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

fn dirs_cache_path(model_id: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let model_dir = model_id.replace('/', "--");
    Some(
        std::path::PathBuf::from(home)
            .join(".cache/huggingface/hub")
            .join(format!("models--{model_dir}")),
    )
}
