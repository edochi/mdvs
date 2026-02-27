use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::index::chunk::{extract_plain_text, Chunks};
use crate::index::embed::{resolve_revision, Embedder, ModelConfig};
use crate::index::storage::{
    build_chunks_batch, build_files_batch, read_parquet, write_parquet, ChunkRow, FileRow,
};
use crate::schema::config::MdvsToml;
use crate::schema::lock::{content_hash, LockFile, MdvsLock};
use datafusion::arrow::datatypes::DataType;
use std::path::Path;

pub fn run(path: &Path) -> anyhow::Result<()> {
    let config_path = path.join("mdvs.toml");
    let lock_path = path.join("mdvs.lock");
    let mdvs_dir = path.join(".mdvs");
    let files_parquet = mdvs_dir.join("files.parquet");
    let chunks_parquet = mdvs_dir.join("chunks.parquet");

    // Read config and lock
    let config = MdvsToml::read(&config_path)?;
    let mut lock = MdvsLock::read(&lock_path)?;

    // Model name check
    anyhow::ensure!(
        config.model.name == lock.model.name,
        "model mismatch: config has '{}' but lock has '{}' (run `mdvs init --force` to reinitialize)",
        config.model.name,
        lock.model.name,
    );

    // Pre-load revision check (if config pins a revision)
    if let (Some(config_rev), Some(lock_rev)) =
        (&config.model.revision, &lock.model.revision)
    {
        anyhow::ensure!(
            config_rev == lock_rev,
            "model revision mismatch: config pins '{}' but lock has '{}' (run `mdvs init --force` to reinitialize)",
            config_rev,
            lock_rev,
        );
    }

    // Load model
    eprintln!("Loading model {}...", config.model.name);
    let model_config = ModelConfig::Model2Vec {
        model_id: config.model.name.clone(),
        revision: config.model.revision.clone(),
    };
    let embedder = Embedder::load(&model_config);
    let dimension = embedder.dimension();

    // Post-load revision check
    if let (Some(resolved), Some(lock_rev)) =
        (resolve_revision(&config.model.name), &lock.model.revision)
    {
        anyhow::ensure!(
            &resolved == lock_rev,
            "model revision mismatch: downloaded '{}' but lock has '{}' (run `mdvs init --force` to reinitialize)",
            resolved,
            lock_rev,
        );
    }

    // Dimension check against existing Parquet
    if chunks_parquet.exists() {
        let batches = read_parquet(&chunks_parquet)?;
        if let Some(batch) = batches.first()
            && let Ok(field) = batch.schema().field_with_name("embedding")
            && let DataType::FixedSizeList(_, existing_dim) = field.data_type()
        {
            let model_dim = dimension as i32;
            anyhow::ensure!(
                *existing_dim == model_dim,
                "dimension mismatch: model produces {model_dim}-dim embeddings but existing index has {existing_dim}-dim",
            );
        }
    }

    // Convert schema fields
    let schema_fields: Vec<(String, FieldType)> = config
        .fields
        .iter()
        .map(|f| {
            let ft = FieldType::try_from(&f.field_type)
                .map_err(|e| anyhow::anyhow!("invalid field type for '{}': {}", f.name, e))?;
            Ok((f.name.clone(), ft))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Scan files
    let scanned = ScannedFiles::scan(
        path,
        &config.config.glob,
        config.config.include_bare_files,
    );

    anyhow::ensure!(
        !scanned.files.is_empty(),
        "no markdown files found in '{}'",
        path.display()
    );

    let max_chunk_size = config.chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    let mut file_rows: Vec<FileRow> = Vec::new();
    let mut chunk_rows: Vec<ChunkRow> = Vec::new();

    for file in &scanned.files {
        let file_id = uuid::Uuid::new_v4().to_string();

        // Chunk the file content
        let chunks = Chunks::new(&file.content, max_chunk_size);

        // Extract plain text from each chunk and embed
        let plain_texts: Vec<String> = chunks.iter().map(|c| extract_plain_text(&c.plain_text)).collect();
        let text_refs: Vec<&str> = plain_texts.iter().map(|s| s.as_str()).collect();
        let embeddings = if text_refs.is_empty() {
            vec![]
        } else {
            embedder.embed_batch(&text_refs)
        };

        // Build file row
        file_rows.push(FileRow {
            file_id: file_id.clone(),
            filename: file.path.display().to_string(),
            frontmatter: file.data.clone(),
            content_hash: content_hash(&file.content),
            built_at,
        });

        // Build chunk rows
        for (chunk, embedding) in chunks.iter().zip(embeddings) {
            chunk_rows.push(ChunkRow {
                chunk_id: uuid::Uuid::new_v4().to_string(),
                file_id: file_id.clone(),
                chunk_index: chunk.chunk_index as i32,
                start_line: chunk.start_line as i32,
                end_line: chunk.end_line as i32,
                embedding,
            });
        }
    }

    // Write Parquet files
    std::fs::create_dir_all(&mdvs_dir)?;

    let files_batch = build_files_batch(&schema_fields, &file_rows);
    write_parquet(&files_parquet, &files_batch)?;

    let chunks_batch = build_chunks_batch(&chunk_rows, dimension as i32);
    write_parquet(&chunks_parquet, &chunks_batch)?;

    // Update lock file hashes
    lock.files = scanned
        .files
        .iter()
        .map(|f| LockFile {
            path: f.path.display().to_string(),
            content_hash: content_hash(&f.content),
        })
        .collect();
    lock.write(&lock_path)?;

    eprintln!(
        "Built index: {} files, {} chunks (dim={})",
        file_rows.len(),
        chunk_rows.len(),
        dimension,
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::shared::{ChunkingConfig, ModelInfo, TomlConfig};
    use std::fs;

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text about the world.",
        )
        .unwrap();
    }

    fn write_config(dir: &Path, model_name: &str, revision: Option<&str>) {
        let config = MdvsToml {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: model_name.into(),
                revision: revision.map(|s| s.into()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            fields: vec![],
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn write_lock(dir: &Path, model_name: &str, revision: Option<&str>) {
        let lock = MdvsLock {
            config: TomlConfig {
                glob: "**".into(),
                include_bare_files: false,
            },
            model: ModelInfo {
                name: model_name.into(),
                revision: revision.map(|s| s.into()),
            },
            chunking: ChunkingConfig {
                max_chunk_size: 1024,
            },
            files: vec![],
            fields: vec![],
        };
        lock.write(&dir.join("mdvs.lock")).unwrap();
    }

    #[test]
    fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn missing_lock() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model", None);
        let result = run(tmp.path());
        assert!(result.is_err());
    }

    #[test]
    fn model_name_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "model-a", None);
        write_lock(tmp.path(), "model-b", None);

        let result = run(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("model mismatch"));
    }

    #[test]
    fn pinned_revision_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        write_config(tmp.path(), "test-model", Some("rev-a"));
        write_lock(tmp.path(), "test-model", Some("rev-b"));

        let result = run(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("revision mismatch"));
    }

    #[test]
    fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init first
        crate::cmd::init::run(
            tmp.path(),
            "minishlab/potion-base-8M",
            None,
            "**",
            false,
            false,
            true, // ignore bare files
            1024,
        )
        .unwrap();

        // Run build
        let result = run(tmp.path());
        assert!(result.is_ok(), "build failed: {:?}", result);

        // Verify Parquet files exist
        let files_path = tmp.path().join(".mdvs/files.parquet");
        let chunks_path = tmp.path().join(".mdvs/chunks.parquet");
        assert!(files_path.exists());
        assert!(chunks_path.exists());

        // Verify files.parquet row count
        let file_batches = read_parquet(&files_path).unwrap();
        let file_rows: usize = file_batches.iter().map(|b| b.num_rows()).sum();
        assert_eq!(file_rows, 2); // 2 files with frontmatter

        // Verify chunks.parquet has embeddings with correct dimension
        let chunk_batches = read_parquet(&chunks_path).unwrap();
        let chunk_rows: usize = chunk_batches.iter().map(|b| b.num_rows()).sum();
        assert!(chunk_rows > 0);

        let embedding_field = chunk_batches[0]
            .schema()
            .field_with_name("embedding")
            .unwrap()
            .clone();
        if let DataType::FixedSizeList(_, dim) = embedding_field.data_type() {
            assert!(*dim > 0);
        } else {
            panic!("expected FixedSizeList for embedding column");
        }

        // Verify lock file was updated with file hashes
        let lock = MdvsLock::read(&tmp.path().join("mdvs.lock")).unwrap();
        assert_eq!(lock.files.len(), 2);
        for f in &lock.files {
            assert!(!f.content_hash.is_empty());
        }
    }

    #[test]
    fn dimension_mismatch() {
        use crate::index::storage::{build_chunks_batch, write_parquet, ChunkRow};

        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init + build
        crate::cmd::init::run(
            tmp.path(),
            "minishlab/potion-base-8M",
            None,
            "**",
            false,
            false,
            true,
            1024,
        )
        .unwrap();
        run(tmp.path()).unwrap();

        // Overwrite chunks.parquet with wrong dimension (2 instead of actual)
        let bad_chunks = vec![ChunkRow {
            chunk_id: "bad".into(),
            file_id: "bad".into(),
            chunk_index: 0,
            start_line: 1,
            end_line: 1,
            embedding: vec![0.1, 0.2], // dim=2
        }];
        let bad_batch = build_chunks_batch(&bad_chunks, 2);
        write_parquet(&tmp.path().join(".mdvs/chunks.parquet"), &bad_batch).unwrap();

        // Build again should fail with dimension mismatch
        let result = run(tmp.path());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("dimension mismatch"));
    }
}
