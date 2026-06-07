//! Indexing pipeline: chunking, embedding, storage, and backend dispatch.

/// LanceDB-backed storage with native vector / FTS / hybrid search.
pub mod backend;
/// Semantic chunking of markdown content.
pub mod chunk;
/// Embedding model loading and inference.
pub mod embed;
/// Arrow batch construction and per-build metadata for the Lance dataset.
pub mod storage;
