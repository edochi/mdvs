//! Indexing pipeline: chunking, embedding, storage, and backend dispatch.

/// Storage backend abstraction (Parquet, future LanceDB).
pub mod backend;
/// Semantic chunking of markdown content.
pub mod chunk;
/// Embedding model loading and inference.
pub mod embed;
/// Parquet I/O for files and chunks.
pub mod storage;
