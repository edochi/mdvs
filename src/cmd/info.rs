use crate::discover::scan::ScannedFiles;
use crate::index::storage::{read_build_metadata, read_parquet};
use crate::output::CommandOutput;
use crate::schema::config::MdvsToml;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct InfoField {
    pub name: String,
    pub field_type: String,
    pub allowed: Vec<String>,
    pub required: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IndexInfo {
    pub model: String,
    pub revision: Option<String>,
    pub chunk_size: usize,
    pub files_indexed: usize,
    pub chunks: usize,
    pub built_at: String,
    pub config_match: bool,
}

#[derive(Debug, Serialize)]
pub struct InfoResult {
    pub scan_glob: String,
    pub files_on_disk: usize,
    pub fields: Vec<InfoField>,
    pub ignored_fields: Vec<String>,
    pub index: Option<IndexInfo>,
}

impl CommandOutput for InfoResult {
    fn format_human(&self) -> String {
        let mut out = String::new();

        // Scan section
        out.push_str(&format!(
            "Scan: glob = \"{}\", {} files on disk\n",
            self.scan_glob, self.files_on_disk,
        ));

        // Fields section
        if !self.fields.is_empty() {
            out.push_str("\nFields:\n");
            for f in &self.fields {
                let mut constraints = Vec::new();
                if !f.required.is_empty() {
                    constraints.push(format!("required in {:?}", f.required));
                }
                if f.allowed != vec!["**"] {
                    constraints.push(format!("allowed in {:?}", f.allowed));
                }
                if constraints.is_empty() {
                    out.push_str(&format!("  {}    {}\n", f.name, f.field_type));
                } else {
                    out.push_str(&format!(
                        "  {}    {}    ({})\n",
                        f.name,
                        f.field_type,
                        constraints.join(", "),
                    ));
                }
            }
        }

        if !self.ignored_fields.is_empty() {
            out.push_str(&format!("Ignored: {}\n", self.ignored_fields.join(", ")));
        }

        // Index section
        out.push('\n');
        match &self.index {
            None => {
                out.push_str("No index built — run 'mdvs build' to create one\n");
            }
            Some(idx) => {
                out.push_str("Index:\n");
                out.push_str(&format!("  Model: {}\n", idx.model));
                if let Some(ref rev) = idx.revision {
                    out.push_str(&format!("  Revision: {rev}\n"));
                }
                out.push_str(&format!("  Chunk size: {}\n", idx.chunk_size));
                out.push_str(&format!(
                    "  {} files indexed, {} chunks\n",
                    idx.files_indexed, idx.chunks,
                ));
                out.push_str(&format!("  Built: {}\n", idx.built_at));
                if idx.config_match {
                    out.push_str("  Status: up to date\n");
                } else {
                    out.push_str("  Status: config changed — rebuild recommended\n");
                }
            }
        }

        out
    }
}

pub fn run(path: &Path) -> anyhow::Result<InfoResult> {
    let config = MdvsToml::read(&path.join("mdvs.toml"))?;

    // Scan file count
    let scanned = ScannedFiles::scan(path, &config.scan);

    // Fields from toml
    let fields: Vec<InfoField> = config
        .fields
        .field
        .iter()
        .map(|f| InfoField {
            name: f.name.clone(),
            field_type: f.field_type.to_string(),
            allowed: f.allowed.clone(),
            required: f.required.clone(),
        })
        .collect();

    // Index info (if .mdvs/ exists)
    let files_parquet = path.join(".mdvs/files.parquet");
    let chunks_parquet = path.join(".mdvs/chunks.parquet");

    let index = if files_parquet.exists() && chunks_parquet.exists() {
        let build_meta = read_build_metadata(&files_parquet)?;
        let file_batches = read_parquet(&files_parquet)?;
        let chunk_batches = read_parquet(&chunks_parquet)?;
        let files_indexed: usize = file_batches.iter().map(|b| b.num_rows()).sum();
        let chunks: usize = chunk_batches.iter().map(|b| b.num_rows()).sum();

        build_meta.map(|meta| {
            let config_match = config.embedding_model.as_ref() == Some(&meta.embedding_model)
                && config.chunking.as_ref() == Some(&meta.chunking);
            IndexInfo {
                model: meta.embedding_model.name,
                revision: meta.embedding_model.revision,
                chunk_size: meta.chunking.max_chunk_size,
                files_indexed,
                chunks,
                built_at: meta.built_at,
                config_match,
            }
        })
    } else {
        None
    };

    Ok(InfoResult {
        scan_glob: config.scan.glob.clone(),
        files_on_disk: scanned.files.len(),
        fields,
        ignored_fields: config.fields.ignore.clone(),
        index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, SearchConfig, UpdateConfig};
    use crate::schema::shared::{ChunkingConfig, EmbeddingModelConfig, FieldTypeSerde, ScanConfig};
    use std::fs;

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Rust Programming\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Rust Programming\nBody.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: Cooking Recipes\ndraft: true\n---\n# Cooking Recipes\nBody.",
        )
        .unwrap();
    }

    fn write_config(dir: &Path) {
        let config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig { auto_build: true },
            fields: FieldsConfig {
                ignore: vec!["internal_id".into()],
                field: vec![
                    crate::schema::config::TomlField {
                        name: "title".into(),
                        field_type: FieldTypeSerde::Scalar("String".into()),
                        allowed: vec!["**".into()],
                        required: vec!["**".into()],
                    },
                    crate::schema::config::TomlField {
                        name: "tags".into(),
                        field_type: FieldTypeSerde::Array {
                            array: Box::new(FieldTypeSerde::Scalar("String".into())),
                        },
                        allowed: vec!["blog/**".into()],
                        required: vec![],
                    },
                    crate::schema::config::TomlField {
                        name: "draft".into(),
                        field_type: FieldTypeSerde::Scalar("Boolean".into()),
                        allowed: vec!["**".into()],
                        required: vec![],
                    },
                ],
            },
            embedding_model: Some(EmbeddingModelConfig {
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            }),
            chunking: Some(ChunkingConfig {
                max_chunk_size: 1024,
            }),
            search: Some(SearchConfig { default_limit: 10 }),
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn init_and_build(dir: &Path) {
        crate::cmd::init::run(
            dir,
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
    }

    #[test]
    fn info_no_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        write_config(tmp.path());

        let result = run(tmp.path()).unwrap();

        assert_eq!(result.scan_glob, "**");
        assert_eq!(result.files_on_disk, 2);
        assert_eq!(result.fields.len(), 3);
        assert_eq!(result.fields[0].name, "title");
        assert_eq!(result.fields[0].field_type, "String");
        assert_eq!(result.fields[1].name, "tags");
        assert_eq!(result.fields[1].field_type, "String[]");
        assert_eq!(result.fields[2].name, "draft");
        assert_eq!(result.fields[2].field_type, "Boolean");
        assert_eq!(result.ignored_fields, vec!["internal_id"]);
        assert!(result.index.is_none());
    }

    #[test]
    fn info_with_index() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path());

        let result = run(tmp.path()).unwrap();

        assert_eq!(result.files_on_disk, 2);
        assert!(result.index.is_some());
        let idx = result.index.unwrap();
        assert_eq!(idx.model, "minishlab/potion-base-8M");
        assert_eq!(idx.files_indexed, 2);
        assert!(idx.chunks > 0);
        assert!(idx.config_match);
    }

    #[test]
    fn info_config_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        init_and_build(tmp.path());

        // Change chunk_size in toml
        let mut config = MdvsToml::read(&tmp.path().join("mdvs.toml")).unwrap();
        config.chunking.as_mut().unwrap().max_chunk_size = 512;
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let result = run(tmp.path()).unwrap();

        assert!(result.index.is_some());
        let idx = result.index.unwrap();
        assert!(!idx.config_match);
    }
}
