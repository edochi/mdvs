use crate::discover::field_type::FieldType;
use crate::discover::scan::ScannedFiles;
use crate::index::chunk::{extract_plain_text, Chunks};
use crate::index::embed::{Embedder, ModelConfig};
use crate::index::storage::{
    build_chunks_batch, build_files_batch, content_hash, read_build_metadata, read_parquet,
    write_parquet, write_parquet_with_metadata, BuildMetadata, ChunkRow, FileRow,
};
use crate::schema::config::{MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use datafusion::arrow::datatypes::DataType;
use std::path::Path;

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

pub fn run(
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
) -> anyhow::Result<()> {
    let config_path = path.join("mdvs.toml");
    let mdvs_dir = path.join(".mdvs");
    let files_parquet = mdvs_dir.join("files.parquet");
    let chunks_parquet = mdvs_dir.join("chunks.parquet");

    // Read config and fill missing build sections
    let mut config = MdvsToml::read(&config_path)?;
    let mut config_changed = false;

    match config.embedding_model {
        None => {
            config.embedding_model = Some(EmbeddingModelConfig {
                name: set_model.unwrap_or(DEFAULT_MODEL).to_string(),
                revision: set_revision.map(|s| s.to_string()),
            });
            config_changed = true;
        }
        Some(ref mut em) if set_model.is_some() || set_revision.is_some() => {
            anyhow::ensure!(
                force,
                "--set-model/--set-revision require --force (changes model, triggers full re-embed)"
            );
            if let Some(m) = set_model {
                em.name = m.to_string();
            }
            if let Some(r) = set_revision {
                em.revision = Some(r.to_string());
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
        Some(ref mut ch) if set_chunk_size.is_some() => {
            anyhow::ensure!(
                force,
                "--set-chunk-size requires --force (changes chunking, triggers full re-embed)"
            );
            ch.max_chunk_size = set_chunk_size.unwrap();
            config_changed = true;
        }
        Some(_) => {}
    }

    if config.search.is_none() {
        config.search = Some(SearchConfig { default_limit: 10 });
        config_changed = true;
    }

    if config_changed {
        config.write(&config_path)?;
    }

    let embedding = config.embedding_model.as_ref().unwrap();
    let chunking = config.chunking.as_ref().unwrap();

    // Detect manual config changes against existing index
    if files_parquet.exists()
        && let Some(ref meta) = read_build_metadata(&files_parquet)?
    {
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
        if !mismatches.is_empty() {
            anyhow::ensure!(
                force,
                "config changed since last build:\n  {}\nUse --force to rebuild with new config",
                mismatches.join("\n  "),
            );
        }
    }

    // Load model
    eprintln!("Loading model {}...", embedding.name);
    let model_config = ModelConfig::Model2Vec {
        model_id: embedding.name.clone(),
        revision: embedding.revision.clone(),
    };
    let embedder = Embedder::load(&model_config);
    let dimension = embedder.dimension();

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
        .field
        .iter()
        .map(|f| {
            let ft = FieldType::try_from(&f.field_type)
                .map_err(|e| anyhow::anyhow!("invalid field type for '{}': {}", f.name, e))?;
            Ok((f.name.clone(), ft))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    // Scan files
    let scanned = ScannedFiles::scan(path, &config.scan);

    anyhow::ensure!(
        !scanned.files.is_empty(),
        "no markdown files found in '{}'",
        path.display()
    );

    let max_chunk_size = chunking.max_chunk_size;
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
    let build_meta = BuildMetadata {
        embedding_model: embedding.clone(),
        chunking: chunking.clone(),
        glob: config.scan.glob.clone(),
        built_at: chrono::Utc::now().to_rfc3339(),
    };
    write_parquet_with_metadata(&files_parquet, &files_batch, build_meta.to_hash_map())?;

    let chunks_batch = build_chunks_batch(&chunk_rows, dimension as i32);
    write_parquet(&chunks_parquet, &chunks_batch)?;

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

    #[test]
    fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run(tmp.path(), None, None, None, false);
        assert!(result.is_err());
    }

    #[test]
    fn end_to_end() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (auto_build calls build internally)
        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true, // ignore bare files
            None,
            true,
            false, // skip_gitignore
        )
        .unwrap();

        // Run build again (tests standalone rebuild)
        let result = run(tmp.path(), None, None, None, false);
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

        // Verify build metadata on files.parquet
        let meta = read_build_metadata(&files_path).unwrap();
        assert!(meta.is_some(), "build metadata should be present");
        let meta = meta.unwrap();
        assert_eq!(meta.embedding_model.name, "minishlab/potion-base-8M");
        assert_eq!(meta.chunking.max_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(meta.glob, "**");
    }

    #[test]
    fn dimension_mismatch() {
        use crate::index::storage::{build_chunks_batch, write_parquet, ChunkRow};

        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Run init (auto_build calls build internally)
        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false, // skip_gitignore
        )
        .unwrap();

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
        let result = run(tmp.path(), None, None, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("dimension mismatch"));
    }

    #[test]
    fn missing_build_sections_filled() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init without auto-build (no build sections in toml)
        crate::cmd::init::run(
            tmp.path(),
            None,
            None,
            "**",
            false,
            false,
            true,
            None,
            false, // no auto_build
            false,
        )
        .unwrap();

        // Verify no build sections
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());
        assert!(config.search.is_none());

        // Build should fill defaults and succeed
        let result = run(tmp.path(), None, None, None, false);
        assert!(result.is_ok(), "build failed: {:?}", result);

        // Verify sections were written
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.embedding_model.as_ref().unwrap().name, DEFAULT_MODEL);
        assert!(config.embedding_model.as_ref().unwrap().revision.is_none());
        assert_eq!(config.chunking.as_ref().unwrap().max_chunk_size, DEFAULT_CHUNK_SIZE);
        assert_eq!(config.search.as_ref().unwrap().default_limit, 10);

        // Verify index was created
        assert!(tmp.path().join(".mdvs/files.parquet").exists());
        assert!(tmp.path().join(".mdvs/chunks.parquet").exists());
    }

    #[test]
    fn set_model_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        // Init with auto-build (sections exist)
        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
        )
        .unwrap();

        // Try to change model without --force
        let result = run(tmp.path(), Some("other-model"), None, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--force"));
    }

    #[test]
    fn set_chunk_size_without_force_errors() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
        )
        .unwrap();

        let result = run(tmp.path(), None, None, Some(512), false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--force"));
    }

    #[test]
    fn set_model_with_force() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
        )
        .unwrap();

        // Change chunk size with --force (same model so no dimension mismatch)
        let result = run(tmp.path(), None, None, Some(512), true);
        assert!(result.is_ok(), "build with --force failed: {:?}", result);

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.chunking.as_ref().unwrap().max_chunk_size, 512);
    }

    #[test]
    fn manual_config_change_detected() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        crate::cmd::init::run(
            tmp.path(),
            Some("minishlab/potion-base-8M"),
            None,
            "**",
            false,
            false,
            true,
            None,
            true,
            false,
        )
        .unwrap();

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let result = run(tmp.path(), None, None, None, false);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("config changed since last build"));
        assert!(err.contains("chunk_size"));

        // Build with --force should succeed
        let result = run(tmp.path(), None, None, None, true);
        assert!(result.is_ok(), "build with --force failed: {:?}", result);
    }
}
