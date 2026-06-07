//! Config mutation helpers used by the build pipeline.
//!
//! - [`mutate_config`] fills in missing build sections (`[embedding_model]`,
//!   `[chunking]`, `[search]`, `[build]`) on first build, or applies
//!   `--set-*` flags on subsequent builds (with `--force` gating).
//! - [`detect_config_changes`] compares the in-memory config against the
//!   metadata in the existing Lance index and reports any drift the user
//!   needs to acknowledge with `--force`.
//! - [`normalize_revision`] is the small helper that treats empty / `None`
//!   strings as "no pinned revision".

use crate::index::backend::Backend;
use crate::index::storage::compute_schema_hash;
use crate::schema::config::{BuildConfig, MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use std::path::Path;

// Unused under `cfg(any(test, feature = "testing-mocks"))` since the
// default falls back to the mock embedder in that build flavor.
#[cfg_attr(any(test, feature = "testing-mocks"), allow(dead_code))]
const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
pub(super) const DEFAULT_CHUNK_SIZE: usize = 1024;

/// Normalize a revision string: empty and "None" are treated as unset.
fn normalize_revision(s: &str) -> Option<String> {
    if s.is_empty() || s.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(s.to_string())
    }
}

/// Apply config mutations: fill missing build sections, apply --set-* flags.
/// Returns `Some(error_message)` if a flag requires --force but wasn't given.
pub(crate) fn mutate_config(
    config: &mut MdvsToml,
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
) -> Option<String> {
    let config_path = path.join("mdvs.toml");
    let mut config_changed = false;

    match config.embedding_model {
        None => {
            // With `--features testing-mocks` (CI fast lane), default to the
            // deterministic mock embedder so tests that flow through init →
            // build don't reach for Hugging Face. The feature is off in
            // production builds, so end users always get model2vec.
            #[cfg(any(test, feature = "testing-mocks"))]
            let default = EmbeddingModelConfig {
                provider: "mock".to_string(),
                name: set_model.unwrap_or("mock").to_string(),
                revision: set_revision.and_then(normalize_revision),
                dim: Some(256),
            };
            #[cfg(not(any(test, feature = "testing-mocks")))]
            let default = EmbeddingModelConfig {
                provider: "model2vec".to_string(),
                name: set_model.unwrap_or(DEFAULT_MODEL).to_string(),
                revision: set_revision.and_then(normalize_revision),
                dim: None,
            };
            config.embedding_model = Some(default);
            config_changed = true;
        }
        Some(ref mut em) if set_model.is_some() || set_revision.is_some() => {
            if !force {
                return Some(
                    "--set-model/--set-revision require --force (changes model, triggers full re-embed)"
                        .to_string(),
                );
            }
            if let Some(m) = set_model {
                em.name = m.to_string();
            }
            if let Some(r) = set_revision {
                em.revision = normalize_revision(r);
            }
            config_changed = true;
        }
        Some(_) => {}
    }

    match config.chunking {
        None => {
            config.chunking = Some(ChunkingConfig {
                max_chunk_size: set_chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
            });
            config_changed = true;
        }
        Some(ref mut ch) => {
            if let Some(new_size) = set_chunk_size {
                if !force {
                    return Some(
                        "--set-chunk-size requires --force (changes chunking, triggers full re-embed)"
                            .to_string(),
                    );
                }
                ch.max_chunk_size = new_size;
                config_changed = true;
            }
        }
    }

    if config.search.is_none() {
        config.search = Some(SearchConfig {
            default_limit: 10,
            auto_update: true,
            auto_build: true,
            internal_prefix: String::new(),
            aliases: std::collections::HashMap::new(),
        });
        config_changed = true;
    }

    if config.build.is_none() {
        config.build = Some(BuildConfig { auto_update: true });
        config_changed = true;
    }

    if config_changed && let Err(e) = config.write(&config_path) {
        return Some(format!("failed to write config: {e}"));
    }

    None
}

/// Detect manual config changes against the existing Lance table-level metadata.
/// Returns `Some(error_message)` if config changed and --force not given.
pub(crate) async fn detect_config_changes(
    backend: &Backend,
    embedding: &EmbeddingModelConfig,
    chunking: &ChunkingConfig,
    config: &MdvsToml,
    force: bool,
) -> Option<String> {
    if force {
        return None;
    }
    let meta = match backend.read_metadata().await {
        Ok(Some(m)) => m,
        Ok(None) => return None, // first build, no metadata
        Err(e) => return Some(e.to_string()),
    };

    let mut mismatches = Vec::new();
    if meta.embedding_model != *embedding {
        mismatches.push(format!(
            "model: '{}' (rev {:?}) -> '{}' (rev {:?})",
            meta.embedding_model.name,
            meta.embedding_model.revision,
            embedding.name,
            embedding.revision,
        ));
    }
    if meta.chunking != *chunking {
        mismatches.push(format!(
            "chunk_size: {} -> {}",
            meta.chunking.max_chunk_size, chunking.max_chunk_size,
        ));
    }
    let current_schema_hash = compute_schema_hash(config);
    if meta.schema_hash != current_schema_hash {
        // Don't show raw hashes — they're noise. Show the fact that the
        // schema content changed, the user knows what they edited.
        mismatches.push(
            "schema: fields, types, constraints, path-scoping, or preprocessors have changed"
                .into(),
        );
    }

    if mismatches.is_empty() {
        None
    } else {
        Some(format!(
            "config changed since last build:\n  {}\nUse --force to rebuild with new config",
            mismatches.join("\n  "),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_revision_clears_empty_and_none() {
        assert_eq!(normalize_revision(""), None);
        assert_eq!(normalize_revision("None"), None);
        assert_eq!(normalize_revision("none"), None);
        assert_eq!(normalize_revision("NONE"), None);
        assert_eq!(normalize_revision("abc123"), Some("abc123".to_string()));
        assert_eq!(normalize_revision("not_none"), Some("not_none".to_string()));
    }
}
