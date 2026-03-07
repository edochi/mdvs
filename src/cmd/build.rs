use crate::discover::field_type::FieldType;
use crate::discover::scan::{ScannedFile, ScannedFiles};
use crate::index::backend::Backend;
use crate::index::chunk::{extract_plain_text, Chunks};
use crate::index::embed::{Embedder, ModelConfig};
use crate::index::storage::{content_hash, BuildMetadata, ChunkRow, FileIndexEntry, FileRow};
use crate::output::{format_file_count, CommandOutput, NewField};
use crate::schema::config::{MdvsToml, SearchConfig};
use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig};
use crate::table::{style_compact, style_record, Builder};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Instant;
use tracing::{info, info_span, instrument};

const DEFAULT_MODEL: &str = "minishlab/potion-base-8M";
const DEFAULT_CHUNK_SIZE: usize = 1024;

// ============================================================================
// BuildResult
// ============================================================================

/// Per-file chunk count for verbose build output.
#[derive(Debug, Serialize)]
pub struct BuildFileDetail {
    /// Relative path of the file.
    pub filename: String,
    /// Number of chunks produced for this file.
    pub chunks: usize,
}

/// Result of the `build` command: embedding and index statistics.
#[derive(Debug, Serialize)]
pub struct BuildResult {
    /// Whether this was a full rebuild (vs incremental).
    pub full_rebuild: bool,
    /// Total number of files in the final index.
    pub files_total: usize,
    /// Number of files that were chunked and embedded this run.
    pub files_embedded: usize,
    /// Number of files reused from the previous index (content unchanged).
    pub files_unchanged: usize,
    /// Number of files removed since the last build.
    pub files_removed: usize,
    /// Total number of chunks in the final index.
    pub chunks_total: usize,
    /// Number of chunks produced by newly embedded files.
    pub chunks_embedded: usize,
    /// Number of chunks retained from unchanged files.
    pub chunks_unchanged: usize,
    /// Number of chunks dropped from removed files.
    pub chunks_removed: usize,
    /// Fields found in frontmatter but not yet in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
    /// Per-file chunk counts for embedded files (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedded_files: Option<Vec<BuildFileDetail>>,
    /// Per-file chunk counts for removed files (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_files: Option<Vec<BuildFileDetail>>,
    /// Embedding model name (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Scan glob pattern (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glob: Option<String>,
    /// Wall-clock time for the build operation in milliseconds (verbose only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_ms: Option<u64>,
}

fn format_chunk_count(n: usize) -> String {
    if n == 1 {
        "1 chunk".to_string()
    } else {
        format!("{n} chunks")
    }
}

impl CommandOutput for BuildResult {
    fn format_text(&self, verbose: bool) -> String {
        let mut out = String::new();

        // New fields (shown before stats)
        for nf in &self.new_fields {
            out.push_str(&format!(
                "  new field: {} ({})\n",
                nf.name,
                format_file_count(nf.files_found)
            ));
        }
        if !self.new_fields.is_empty() {
            out.push_str("Run 'mdvs update' to incorporate new fields.\n\n");
        }

        // One-liner
        let rebuild_suffix = if self.full_rebuild {
            " (full rebuild)"
        } else {
            ""
        };
        out.push_str(&format!(
            "Built index — {}, {}{rebuild_suffix}\n",
            format_file_count(self.files_total),
            format_chunk_count(self.chunks_total)
        ));

        // Stats table
        out.push('\n');
        if verbose {
            // Verbose: record tables for embedded/removed, compact for unchanged
            if self.files_embedded > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "embedded".to_string(),
                    format_file_count(self.files_embedded),
                    format_chunk_count(self.chunks_embedded),
                ]);
                let detail = match &self.embedded_files {
                    Some(files) => {
                        let lines: Vec<String> = files
                            .iter()
                            .map(|f| {
                                format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks))
                            })
                            .collect();
                        lines.join("\n")
                    }
                    None => String::new(),
                };
                builder.push_record([detail, String::new(), String::new()]);
                let mut table = builder.build();
                style_record(&mut table, 3);
                out.push_str(&format!("{table}\n"));
            }
            if self.files_unchanged > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "unchanged".to_string(),
                    format_file_count(self.files_unchanged),
                    format_chunk_count(self.chunks_unchanged),
                ]);
                let mut table = builder.build();
                style_compact(&mut table);
                out.push_str(&format!("{table}\n"));
            }
            if self.files_removed > 0 {
                let mut builder = Builder::default();
                builder.push_record([
                    "removed".to_string(),
                    format_file_count(self.files_removed),
                    format_chunk_count(self.chunks_removed),
                ]);
                let detail = match &self.removed_files {
                    Some(files) => {
                        let lines: Vec<String> = files
                            .iter()
                            .map(|f| {
                                format!("  - \"{}\" ({})", f.filename, format_chunk_count(f.chunks))
                            })
                            .collect();
                        lines.join("\n")
                    }
                    None => String::new(),
                };
                builder.push_record([detail, String::new(), String::new()]);
                let mut table = builder.build();
                style_record(&mut table, 3);
                out.push_str(&format!("{table}\n"));
            }
        } else {
            // Compact: single table with all non-zero categories
            let mut builder = Builder::default();
            if self.files_embedded > 0 {
                builder.push_record([
                    "embedded".to_string(),
                    format_file_count(self.files_embedded),
                    format_chunk_count(self.chunks_embedded),
                ]);
            }
            if self.files_unchanged > 0 {
                builder.push_record([
                    "unchanged".to_string(),
                    format_file_count(self.files_unchanged),
                    format_chunk_count(self.chunks_unchanged),
                ]);
            }
            if self.files_removed > 0 {
                builder.push_record([
                    "removed".to_string(),
                    format_file_count(self.files_removed),
                    format_chunk_count(self.chunks_removed),
                ]);
            }
            let mut table = builder.build();
            style_compact(&mut table);
            out.push_str(&format!("{table}\n"));
        }

        // Verbose footer
        if verbose {
            if let (Some(model), Some(glob), Some(ms)) = (&self.model, &self.glob, self.elapsed_ms)
            {
                out.push_str(&format!(
                    "{} | model: \"{model}\" | glob: \"{glob}\" | {ms}ms\n",
                    format_file_count(self.files_total)
                ));
            }
        }

        out
    }
}

// ============================================================================
// File classification for incremental build
// ============================================================================

struct FileClassification<'a> {
    /// Files that need chunking + embedding (new or edited).
    needs_embedding: Vec<FileToEmbed<'a>>,
    /// Maps filename → file_id for ALL current files (new, edited, unchanged).
    file_id_map: HashMap<String, String>,
    /// file_ids whose existing chunks should be retained.
    unchanged_file_ids: HashSet<String>,
    /// Number of files in the old index that no longer exist.
    removed_count: usize,
    /// file_ids of removed files (for chunk counting).
    removed_file_ids: HashSet<String>,
    /// Filenames of removed files (for verbose output).
    removed_filenames: Vec<String>,
}

struct FileToEmbed<'a> {
    file_id: String,
    scanned: &'a ScannedFile,
}

#[instrument(name = "classify", skip_all, level = "debug")]
fn classify_files<'a>(
    scanned: &'a ScannedFiles,
    existing_index: &[FileIndexEntry],
) -> FileClassification<'a> {
    let existing: HashMap<&str, (&str, &str)> = existing_index
        .iter()
        .map(|e| {
            (
                e.filename.as_str(),
                (e.file_id.as_str(), e.content_hash.as_str()),
            )
        })
        .collect();

    let mut needs_embedding = Vec::new();
    let mut file_id_map = HashMap::new();
    let mut unchanged_file_ids = HashSet::new();
    let mut seen_filenames = HashSet::new();

    for file in &scanned.files {
        let filename = file.path.display().to_string();
        let hash = content_hash(&file.content);

        if let Some(&(old_id, old_hash)) = existing.get(filename.as_str()) {
            seen_filenames.insert(filename.clone());
            if hash == old_hash {
                // Unchanged — keep existing chunks
                file_id_map.insert(filename, old_id.to_string());
                unchanged_file_ids.insert(old_id.to_string());
            } else {
                // Edited — re-embed, keep file_id
                let file_id = old_id.to_string();
                file_id_map.insert(filename, file_id.clone());
                needs_embedding.push(FileToEmbed {
                    file_id,
                    scanned: file,
                });
            }
        } else {
            // New file
            let file_id = uuid::Uuid::new_v4().to_string();
            file_id_map.insert(filename, file_id.clone());
            needs_embedding.push(FileToEmbed {
                file_id,
                scanned: file,
            });
        }
    }

    let mut removed_file_ids = HashSet::new();
    let mut removed_filenames = Vec::new();
    for entry in existing_index {
        if !seen_filenames.contains(entry.filename.as_str()) {
            removed_file_ids.insert(entry.file_id.clone());
            removed_filenames.push(entry.filename.clone());
        }
    }
    let removed_count = removed_filenames.len();

    FileClassification {
        needs_embedding,
        file_id_map,
        unchanged_file_ids,
        removed_count,
        removed_file_ids,
        removed_filenames,
    }
}

/// Load the embedding model and verify its dimension matches any existing index.
fn load_embedder(embedding: &EmbeddingModelConfig, backend: &Backend) -> anyhow::Result<Embedder> {
    info!(model = %embedding.name, "loading model");
    let t = Instant::now();
    let model_config = ModelConfig::try_from(embedding)?;
    let embedder = Embedder::load(&model_config)?;
    info!(elapsed_ms = t.elapsed().as_millis() as u64, "model loaded");

    if let Some(existing_dim) = backend.embedding_dimension()? {
        let model_dim = embedder.dimension() as i32;
        anyhow::ensure!(
            existing_dim == model_dim,
            "dimension mismatch: model produces {model_dim}-dim embeddings but existing index has {existing_dim}-dim",
        );
    }

    Ok(embedder)
}

/// Validate frontmatter, chunk, embed, and write Parquet files to `.mdvs/`.
#[instrument(name = "build", skip_all)]
pub async fn run(
    path: &Path,
    set_model: Option<&str>,
    set_revision: Option<&str>,
    set_chunk_size: Option<usize>,
    force: bool,
    verbose: bool,
) -> anyhow::Result<BuildResult> {
    let start = Instant::now();
    let config_path = path.join("mdvs.toml");

    // Read config and fill missing build sections
    let mut config = MdvsToml::read(&config_path)?;
    let mut config_changed = false;

    match config.embedding_model {
        None => {
            config.embedding_model = Some(EmbeddingModelConfig {
                provider: "model2vec".to_string(),
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

    let backend = Backend::parquet(path, config.internal_prefix());

    // Detect manual config changes against existing index
    if let Some(ref meta) = backend.read_metadata()? {
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
        if meta.internal_prefix != config.internal_prefix() {
            mismatches.push(format!(
                "internal_prefix: '{}' -> '{}'",
                meta.internal_prefix,
                config.internal_prefix(),
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

    // Scan files
    let scanned = ScannedFiles::scan(path, &config.scan)?;

    anyhow::ensure!(
        !scanned.files.is_empty(),
        "no markdown files found in '{}'",
        path.display()
    );

    // Validate frontmatter against schema (abort on violations)
    let check_result = crate::cmd::check::validate(&scanned, &config, false)?;
    if check_result.has_violations() {
        let report = crate::output::CommandOutput::format_text(&check_result, false);
        anyhow::bail!("{report}build aborted due to validation errors");
    }
    let new_fields = check_result.new_fields;

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

    let max_chunk_size = chunking.max_chunk_size;
    let built_at = chrono::Utc::now().timestamp_micros();

    let full_rebuild = force || !backend.exists();

    // Track per-file chunk counts for result
    let mut embedded_details: Vec<BuildFileDetail> = Vec::new();
    let mut removed_details: Vec<BuildFileDetail> = Vec::new();
    let mut chunks_removed: usize = 0;

    let (file_rows, chunk_rows, files_embedded, files_unchanged, files_removed) = if full_rebuild {
        // === FULL REBUILD ===
        let embedder = load_embedder(embedding, &backend)?;

        let mut file_rows = Vec::new();
        let mut chunk_rows = Vec::new();

        {
            let t = Instant::now();
            let _span = info_span!("embed", files = scanned.files.len()).entered();
            for file in &scanned.files {
                let file_id = uuid::Uuid::new_v4().to_string();
                let (fr, crs) =
                    embed_file(&file_id, file, max_chunk_size, built_at, &embedder).await;
                embedded_details.push(BuildFileDetail {
                    filename: file.path.display().to_string(),
                    chunks: crs.len(),
                });
                file_rows.push(fr);
                chunk_rows.extend(crs);
            }
            info!(
                elapsed_ms = t.elapsed().as_millis() as u64,
                chunks = chunk_rows.len(),
                "embedding complete"
            );
        }

        let count = scanned.files.len();
        (file_rows, chunk_rows, count, 0, 0)
    } else {
        // === INCREMENTAL BUILD ===
        let existing_index = backend.read_file_index()?;
        let classification = classify_files(&scanned, &existing_index);

        info!(
            to_embed = classification.needs_embedding.len(),
            unchanged = classification.unchanged_file_ids.len(),
            removed = classification.removed_count,
            "files classified"
        );

        // Build file rows for ALL scanned files with fresh frontmatter
        let file_rows: Vec<FileRow> = scanned
            .files
            .iter()
            .map(|f| {
                let filename = f.path.display().to_string();
                let file_id = classification.file_id_map[&filename].clone();
                FileRow {
                    file_id,
                    filename,
                    frontmatter: f.data.clone(),
                    content_hash: content_hash(&f.content),
                    built_at,
                }
            })
            .collect();

        // Count removed chunks and build removed file details
        let existing_chunks = backend.read_chunk_rows()?;
        {
            // Count chunks per removed file
            let mut removed_chunk_counts: HashMap<&str, usize> = HashMap::new();
            for c in &existing_chunks {
                if classification.removed_file_ids.contains(&c.file_id) {
                    *removed_chunk_counts.entry(c.file_id.as_str()).or_default() += 1;
                }
            }
            chunks_removed = removed_chunk_counts.values().sum();

            // Build removed file details (map file_id back to filename)
            let filename_to_id: HashMap<&str, &str> = existing_index
                .iter()
                .map(|e| (e.filename.as_str(), e.file_id.as_str()))
                .collect();
            for filename in &classification.removed_filenames {
                let file_id = filename_to_id.get(filename.as_str()).copied().unwrap_or("");
                let chunk_count = removed_chunk_counts.get(file_id).copied().unwrap_or(0);
                removed_details.push(BuildFileDetail {
                    filename: filename.clone(),
                    chunks: chunk_count,
                });
            }
        }

        // Retain existing chunks for unchanged files
        let mut chunk_rows: Vec<ChunkRow> = existing_chunks
            .into_iter()
            .filter(|c| classification.unchanged_file_ids.contains(&c.file_id))
            .collect();

        if !classification.needs_embedding.is_empty() {
            let embedder = load_embedder(embedding, &backend)?;

            {
                let t = Instant::now();
                let _span =
                    info_span!("embed", files = classification.needs_embedding.len()).entered();
                for fte in &classification.needs_embedding {
                    let (_, crs) = embed_file(
                        &fte.file_id,
                        fte.scanned,
                        max_chunk_size,
                        built_at,
                        &embedder,
                    )
                    .await;
                    embedded_details.push(BuildFileDetail {
                        filename: fte.scanned.path.display().to_string(),
                        chunks: crs.len(),
                    });
                    chunk_rows.extend(crs);
                }
                info!(
                    elapsed_ms = t.elapsed().as_millis() as u64,
                    "embedding complete"
                );
            }
        } else {
            info!("no content changes, skipping embedding");
        }

        let embedded = classification.needs_embedding.len();
        let unchanged = classification.unchanged_file_ids.len();
        let removed = classification.removed_count;
        (file_rows, chunk_rows, embedded, unchanged, removed)
    };

    // Write index
    let build_meta = BuildMetadata {
        embedding_model: embedding.clone(),
        chunking: chunking.clone(),
        glob: config.scan.glob.clone(),
        built_at: chrono::Utc::now().to_rfc3339(),
        internal_prefix: config.internal_prefix().to_string(),
    };
    let t = Instant::now();
    backend.write_index(&schema_fields, &file_rows, &chunk_rows, build_meta)?;
    info!(
        files = file_rows.len(),
        chunks = chunk_rows.len(),
        elapsed_ms = t.elapsed().as_millis() as u64,
        "index written"
    );

    let chunks_embedded: usize = embedded_details.iter().map(|d| d.chunks).sum();
    let chunks_total = chunk_rows.len();
    let chunks_unchanged = chunks_total - chunks_embedded;

    Ok(BuildResult {
        full_rebuild,
        files_total: file_rows.len(),
        files_embedded,
        files_unchanged,
        files_removed,
        chunks_total,
        chunks_embedded,
        chunks_unchanged,
        chunks_removed,
        new_fields,
        embedded_files: if verbose {
            Some(embedded_details)
        } else {
            None
        },
        removed_files: if verbose && !removed_details.is_empty() {
            Some(removed_details)
        } else {
            None
        },
        model: if verbose {
            Some(embedding.name.clone())
        } else {
            None
        },
        glob: if verbose {
            Some(config.scan.glob.clone())
        } else {
            None
        },
        elapsed_ms: if verbose {
            Some(start.elapsed().as_millis() as u64)
        } else {
            None
        },
    })
}

#[instrument(name = "embed_file", skip_all, fields(file = %file.path.display()), level = "debug")]
async fn embed_file(
    file_id: &str,
    file: &ScannedFile,
    max_chunk_size: usize,
    built_at: i64,
    embedder: &Embedder,
) -> (FileRow, Vec<ChunkRow>) {
    let chunks = Chunks::new(&file.content, max_chunk_size);
    let plain_texts: Vec<String> = chunks
        .iter()
        .map(|c| extract_plain_text(&c.plain_text))
        .collect();
    let text_refs: Vec<&str> = plain_texts.iter().map(|s| s.as_str()).collect();
    let embeddings = if text_refs.is_empty() {
        vec![]
    } else {
        embedder.embed_batch(&text_refs).await
    };

    let file_row = FileRow {
        file_id: file_id.to_string(),
        filename: file.path.display().to_string(),
        frontmatter: file.data.clone(),
        content_hash: content_hash(&file.content),
        built_at,
    };

    let chunk_rows: Vec<ChunkRow> = chunks
        .iter()
        .zip(embeddings)
        .map(|(chunk, embedding)| ChunkRow {
            chunk_id: uuid::Uuid::new_v4().to_string(),
            file_id: file_id.to_string(),
            chunk_index: chunk.chunk_index as i32,
            start_line: chunk.start_line as i32,
            end_line: chunk.end_line as i32,
            embedding,
        })
        .collect();

    (file_row, chunk_rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::storage::{read_build_metadata, read_parquet};
    use datafusion::arrow::datatypes::DataType;
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

    #[tokio::test]
    async fn missing_config() {
        let tmp = tempfile::tempdir().unwrap();
        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn end_to_end() {
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
            false, // verbose
        )
        .await
        .unwrap();

        // Run build again (tests standalone rebuild)
        let result = run(tmp.path(), None, None, None, false, false).await;
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
            .field_with_name("_embedding")
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

    #[tokio::test]
    async fn dimension_mismatch() {
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
            false, // verbose
        )
        .await
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
        let bad_batch = build_chunks_batch(&bad_chunks, 2, "_");
        write_parquet(&tmp.path().join(".mdvs/chunks.parquet"), &bad_batch).unwrap();

        // Add a new file so model gets loaded (incremental detects new file)
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: New\ntags:\n  - test\ndraft: true\n---\n# New\nNew content.",
        )
        .unwrap();

        // Build should fail with dimension mismatch when model loads
        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("dimension mismatch"));
    }

    #[tokio::test]
    async fn missing_build_sections_filled() {
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
            false, // verbose
        )
        .await
        .unwrap();

        // Verify no build sections
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert!(config.embedding_model.is_none());
        assert!(config.chunking.is_none());
        assert!(config.search.is_none());

        // Build should fill defaults and succeed
        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_ok(), "build failed: {:?}", result);

        // Verify sections were written
        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.embedding_model.as_ref().unwrap().name, DEFAULT_MODEL);
        assert!(config.embedding_model.as_ref().unwrap().revision.is_none());
        assert_eq!(
            config.chunking.as_ref().unwrap().max_chunk_size,
            DEFAULT_CHUNK_SIZE
        );
        assert_eq!(config.search.as_ref().unwrap().default_limit, 10);

        // Verify index was created
        assert!(tmp.path().join(".mdvs/files.parquet").exists());
        assert!(tmp.path().join(".mdvs/chunks.parquet").exists());
    }

    #[tokio::test]
    async fn set_model_without_force_errors() {
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
            false, // verbose
        )
        .await
        .unwrap();

        // Try to change model without --force
        let result = run(tmp.path(), Some("other-model"), None, None, false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--force"));
    }

    #[tokio::test]
    async fn set_chunk_size_without_force_errors() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let result = run(tmp.path(), None, None, Some(512), false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("--force"));
    }

    #[tokio::test]
    async fn set_model_with_force() {
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
            false, // verbose
        )
        .await
        .unwrap();

        // Change chunk size with --force (same model so no dimension mismatch)
        let result = run(tmp.path(), None, None, Some(512), true, false).await;
        assert!(result.is_ok(), "build with --force failed: {:?}", result);

        let config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        assert_eq!(config.chunking.as_ref().unwrap().max_chunk_size, 512);
    }

    #[tokio::test]
    async fn manual_config_change_detected() {
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
            false, // verbose
        )
        .await
        .unwrap();

        // Manually change chunk_size in toml (simulates user editing)
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 256;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build without --force should error
        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("config changed since last build"));
        assert!(err.contains("chunk_size"));

        // Build with --force should succeed
        let result = run(tmp.path(), None, None, None, true, false).await;
        assert!(result.is_ok(), "build with --force failed: {:?}", result);
    }

    #[tokio::test]
    async fn build_aborts_on_wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // draft is string "yes" in the file but declared as Boolean in toml
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Hello\nBody.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig { auto_build: false },
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        // Declare as Boolean, but file has String → WrongType violation
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                    },
                ],
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("validation errors"),
            "expected validation abort, got: {err}"
        );
    }

    #[tokio::test]
    async fn build_aborts_on_missing_required() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // post1 has tags, post2 does not — tags required in blog/**
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n---\n# Hello\nBody.",
        )
        .unwrap();
        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\n---\n# World\nBody.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig { auto_build: false },
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: crate::schema::shared::FieldTypeSerde::Array {
                            array: Box::new(crate::schema::shared::FieldTypeSerde::Scalar(
                                "String".into(),
                            )),
                        },
                        allowed: vec!["**".into()],
                        required: vec!["blog/**".into()],
                    },
                ],
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("validation errors"),
            "expected validation abort, got: {err}"
        );
    }

    #[tokio::test]
    async fn build_succeeds_with_new_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        // File has title + author, but toml only declares title
        // author is a "new field" — informational, should not block build
        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\nauthor: Alice\n---\n# Hello\nBody text.",
        )
        .unwrap();

        let config = MdvsToml {
            scan: crate::schema::shared::ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: crate::schema::config::UpdateConfig { auto_build: false },
            fields: crate::schema::config::FieldsConfig {
                ignore: vec![],
                field: vec![crate::schema::config::TomlField {
                    name: "title".into(),
                    field_type: crate::schema::shared::FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                }],
            },
            embedding_model: Some(EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
            storage: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        // Build should succeed despite unknown "author" field
        let result = run(tmp.path(), None, None, None, false, false).await;
        assert!(
            result.is_ok(),
            "build should succeed with new fields: {:?}",
            result
        );

        // Verify index was created
        assert!(tmp.path().join(".mdvs/files.parquet").exists());
        assert!(tmp.path().join(".mdvs/chunks.parquet").exists());
    }

    // ========================================================================
    // classify_files unit tests
    // ========================================================================

    fn make_scanned_files(files: Vec<(&str, &str)>) -> ScannedFiles {
        ScannedFiles {
            files: files
                .into_iter()
                .map(|(path, body)| ScannedFile {
                    path: std::path::PathBuf::from(path),
                    data: None,
                    content: body.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn classify_all_new() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing: Vec<FileIndexEntry> = vec![];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 2);
        assert_eq!(c.unchanged_file_ids.len(), 0);
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map.len(), 2);
    }

    #[test]
    fn classify_all_unchanged() {
        let scanned = make_scanned_files(vec![("a.md", "hello"), ("b.md", "world")]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("hello"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("world"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        assert_eq!(c.needs_embedding.len(), 0);
        assert_eq!(c.unchanged_file_ids.len(), 2);
        assert!(c.unchanged_file_ids.contains("f1"));
        assert!(c.unchanged_file_ids.contains("f2"));
        assert_eq!(c.removed_count, 0);
        assert_eq!(c.file_id_map["a.md"], "f1");
        assert_eq!(c.file_id_map["b.md"], "f2");
    }

    #[test]
    fn classify_mixed() {
        // a.md: unchanged, b.md: edited, c.md: new, d.md: removed
        let scanned = make_scanned_files(vec![
            ("a.md", "same content"),
            ("b.md", "new body"),
            ("c.md", "brand new"),
        ]);
        let existing = vec![
            FileIndexEntry {
                file_id: "f1".into(),
                filename: "a.md".into(),
                content_hash: content_hash("same content"),
            },
            FileIndexEntry {
                file_id: "f2".into(),
                filename: "b.md".into(),
                content_hash: content_hash("old body"),
            },
            FileIndexEntry {
                file_id: "f3".into(),
                filename: "d.md".into(),
                content_hash: content_hash("deleted"),
            },
        ];
        let c = classify_files(&scanned, &existing);

        // a.md unchanged
        assert!(c.unchanged_file_ids.contains("f1"));
        assert_eq!(c.file_id_map["a.md"], "f1");

        // b.md edited — needs embedding, keeps file_id
        assert_eq!(c.needs_embedding.len(), 2); // b.md + c.md
        let edited = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("b.md"))
            .unwrap();
        assert_eq!(edited.file_id, "f2");

        // c.md new — needs embedding, new UUID
        let new = c
            .needs_embedding
            .iter()
            .find(|f| f.scanned.path.to_str() == Some("c.md"))
            .unwrap();
        assert_ne!(new.file_id, "f1");
        assert_ne!(new.file_id, "f2");
        assert_ne!(new.file_id, "f3");

        // d.md removed
        assert_eq!(c.removed_count, 1);
        assert!(!c.file_id_map.contains_key("d.md"));
    }

    // ========================================================================
    // Incremental build integration tests
    // ========================================================================

    use crate::index::storage::{read_chunk_rows, read_file_index};

    /// Read file_id→filename map and chunk_id→file_id map from existing parquets.
    fn read_index_state(dir: &Path) -> (HashMap<String, String>, Vec<(String, String)>) {
        let file_index = read_file_index(&dir.join(".mdvs/files.parquet")).unwrap();
        let file_map: HashMap<String, String> = file_index
            .iter()
            .map(|e| (e.filename.clone(), e.file_id.clone()))
            .collect();
        let chunks = read_chunk_rows(&dir.join(".mdvs/chunks.parquet")).unwrap();
        let chunk_pairs: Vec<(String, String)> = chunks
            .iter()
            .map(|c| (c.chunk_id.clone(), c.file_id.clone()))
            .collect();
        (file_map, chunk_pairs)
    }

    #[tokio::test]
    async fn incremental_no_changes() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (files_before, chunks_before) = read_index_state(tmp.path());

        // Build again with no changes
        run(tmp.path(), None, None, None, false, false)
            .await
            .unwrap();

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // chunk_ids preserved (same chunks carried forward)
        let old_chunk_ids: HashSet<&str> =
            chunks_before.iter().map(|(id, _)| id.as_str()).collect();
        let new_chunk_ids: HashSet<&str> = chunks_after.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn incremental_new_file() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (files_before, chunks_before) = read_index_state(tmp.path());
        // Add a new file
        fs::write(
            tmp.path().join("blog/post3.md"),
            "---\ntitle: Third\ntags:\n  - new\ndraft: false\n---\n# Third\nNew post content.",
        )
        .unwrap();

        run(tmp.path(), None, None, None, false, false)
            .await
            .unwrap();

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // Old file_ids preserved
        for (filename, old_id) in &files_before {
            assert_eq!(
                files_after[filename], *old_id,
                "file_id changed for {filename}"
            );
        }
        // New file added
        assert!(files_after.contains_key("blog/post3.md"));
        assert_eq!(files_after.len(), 3);

        // Old chunks preserved, new chunks added
        for (chunk_id, _) in &chunks_before {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "old chunk {chunk_id} missing",
            );
        }
        assert!(chunks_after.len() > chunks_before.len());
    }

    #[tokio::test]
    async fn incremental_edited_file() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let post1_id = files_before["blog/post1.md"].clone();
        let post2_id = files_before["blog/post2.md"].clone();

        // Chunks belonging to each file
        let post1_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        let post2_chunks: HashSet<String> = chunks_before
            .iter()
            .filter(|(_, fid)| fid == &post2_id)
            .map(|(cid, _)| cid.clone())
            .collect();

        // Edit post1's body (keep same frontmatter)
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nCompletely different body text.",
        ).unwrap();

        run(tmp.path(), None, None, None, false, false)
            .await
            .unwrap();

        let (files_after, chunks_after) = read_index_state(tmp.path());

        // file_ids preserved for both files
        assert_eq!(files_after["blog/post1.md"], post1_id);
        assert_eq!(files_after["blog/post2.md"], post2_id);

        // post2 chunks preserved (unchanged file)
        for chunk_id in &post2_chunks {
            assert!(
                chunks_after.iter().any(|(id, _)| id == chunk_id),
                "post2 chunk {chunk_id} should be preserved",
            );
        }
        // post1 chunks replaced (edited file — new chunk_ids)
        let new_post1_chunks: HashSet<String> = chunks_after
            .iter()
            .filter(|(_, fid)| fid == &post1_id)
            .map(|(cid, _)| cid.clone())
            .collect();
        assert!(!new_post1_chunks.is_empty());
        for old_id in &post1_chunks {
            assert!(
                !new_post1_chunks.contains(old_id),
                "old chunk should be replaced"
            );
        }
    }

    #[tokio::test]
    async fn incremental_removed_file() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (files_before, _) = read_index_state(tmp.path());
        assert_eq!(files_before.len(), 2);

        // Remove post2
        fs::remove_file(tmp.path().join("blog/post2.md")).unwrap();

        run(tmp.path(), None, None, None, false, false)
            .await
            .unwrap();

        let (files_after, chunks_after) = read_index_state(tmp.path());

        assert_eq!(files_after.len(), 1);
        assert!(files_after.contains_key("blog/post1.md"));
        assert!(!files_after.contains_key("blog/post2.md"));

        // No chunks referencing removed file
        let post2_id = &files_before["blog/post2.md"];
        assert!(!chunks_after.iter().any(|(_, fid)| fid == post2_id));
    }

    #[tokio::test]
    async fn incremental_frontmatter_only_change() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (_, chunks_before) = read_index_state(tmp.path());
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Change only frontmatter (add a tag), keep same body
        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\n  - new-tag\ndraft: false\n---\n# Hello\nBody text about Rust programming.",
        ).unwrap();

        run(tmp.path(), None, None, None, false, false)
            .await
            .unwrap();

        let (_, chunks_after) = read_index_state(tmp.path());
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // Chunks preserved — body didn't change, no re-embedding
        assert_eq!(old_chunk_ids, new_chunk_ids);
    }

    #[tokio::test]
    async fn force_full_rebuild() {
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
            false, // verbose
        )
        .await
        .unwrap();

        let (files_before, chunks_before) = read_index_state(tmp.path());
        let old_file_ids: HashSet<String> = files_before.values().cloned().collect();
        let old_chunk_ids: HashSet<String> =
            chunks_before.iter().map(|(id, _)| id.clone()).collect();

        // Force rebuild — should generate all new IDs
        run(tmp.path(), None, None, None, true, false)
            .await
            .unwrap();

        let (files_after, chunks_after) = read_index_state(tmp.path());
        let new_file_ids: HashSet<String> = files_after.values().cloned().collect();
        let new_chunk_ids: HashSet<String> =
            chunks_after.iter().map(|(id, _)| id.clone()).collect();

        // All IDs should be different (new UUIDs)
        assert!(
            old_file_ids.is_disjoint(&new_file_ids),
            "force rebuild should generate new file_ids"
        );
        assert!(
            old_chunk_ids.is_disjoint(&new_chunk_ids),
            "force rebuild should generate new chunk_ids"
        );
    }
}
