use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::index::backend::{Backend, IndexStats};
use crate::index::storage::BuildMetadata;
use crate::outcome::{
    InferOutcome, Outcome, ReadConfigOutcome, ReadIndexOutcome, ScanOutcome, WriteConfigOutcome,
};
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::shared::{FieldTypeSerde, ScanConfig};
use crate::step::{ErrorKind, StepEntry, elapsed_ms};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

// Each helper times its own work, pushes exactly one step entry per phase it
// runs, and on failure returns `Err(())` after pushing the error step. The
// caller turns a failure into a `CommandResult` (usually with
// `CommandResult::failed_from_steps`, or with an explicit message where the
// command reports one).

/// Read `<path>/mdvs.toml` and validate it, timed as one step.
///
/// Returns the config and the path it was read from. A read error is reported
/// verbatim; a validation error is wrapped with a hint to fix the file or
/// re-run `mdvs init --force`.
pub(crate) fn read_config_step(
    path: &Path,
    steps: &mut Vec<StepEntry>,
) -> Result<(MdvsToml, PathBuf), ()> {
    let config_start = Instant::now();
    let config_path = path.join("mdvs.toml");
    let message = match MdvsToml::read(&config_path) {
        Ok(cfg) => match cfg.validate() {
            Ok(()) => {
                steps.push(StepEntry::ok(
                    Outcome::ReadConfig(ReadConfigOutcome {
                        config_path: config_path.display().to_string(),
                    }),
                    elapsed_ms(config_start),
                ));
                return Ok((cfg, config_path));
            }
            Err(e) => {
                format!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'")
            }
        },
        Err(e) => e.to_string(),
    };
    steps.push(StepEntry::err(
        ErrorKind::User,
        message,
        elapsed_ms(config_start),
    ));
    Err(())
}

/// Scan `path` for markdown files according to `scan`.
pub(crate) fn scan_step(
    path: &Path,
    scan: &ScanConfig,
    steps: &mut Vec<StepEntry>,
) -> Result<ScannedFiles, ()> {
    let scan_start = Instant::now();
    match ScannedFiles::scan(path, scan) {
        Ok(s) => {
            steps.push(StepEntry::ok(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: scan.glob.clone(),
                }),
                elapsed_ms(scan_start),
            ));
            Ok(s)
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(scan_start),
            ));
            Err(())
        }
    }
}

/// Infer a schema from the scanned files and warn about any dropped fields.
pub(crate) fn infer_step(scanned: &ScannedFiles, steps: &mut Vec<StepEntry>) -> InferredSchema {
    let infer_start = Instant::now();
    let schema = InferredSchema::infer(scanned);
    steps.push(StepEntry::ok(
        Outcome::Infer(InferOutcome {
            fields_inferred: schema.fields.len(),
        }),
        elapsed_ms(infer_start),
    ));
    schema.emit_dropped_warnings();
    schema
}

/// Add fields seen in the scan but absent from `config` (and not ignored),
/// then write and re-read the config.
///
/// Always runs the infer step. The write step runs only when there are new
/// fields. On success `config` holds the re-read config (or the extended
/// in-memory one if the re-read fails). Fails only when the write fails.
pub(crate) fn auto_update_step(
    config: &mut MdvsToml,
    config_path: &Path,
    scanned: &ScannedFiles,
    steps: &mut Vec<StepEntry>,
) -> Result<(), ()> {
    let schema = infer_step(scanned, steps);

    let existing: HashSet<&str> = config
        .fields
        .field
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    let new_toml_fields: Vec<TomlField> = schema
        .fields
        .iter()
        .filter(|f| !existing.contains(f.name.as_str()))
        .filter(|f| !config.fields.ignore.contains(&f.name))
        .inspect(|f| f.emit_inexact_widening_warning())
        .map(|f| TomlField {
            name: f.name.clone(),
            field_type: FieldTypeSerde::from(&f.field_type),
            allowed: f.allowed.clone(),
            required: f.required.clone(),
            nullable: f.nullable,
            constraints: None,
            preprocess: f.preprocess.clone(),
        })
        .collect();

    if new_toml_fields.is_empty() {
        return Ok(());
    }

    config.fields.field.extend(new_toml_fields);
    let write_start = Instant::now();
    match config.write(config_path) {
        Ok(()) => {
            steps.push(StepEntry::ok(
                Outcome::WriteConfig(WriteConfigOutcome {
                    config_path: config_path.display().to_string(),
                    fields_written: config.fields.field.len(),
                }),
                elapsed_ms(write_start),
            ));
            // Re-read to pick up normalized TOML
            if let Ok(c) = MdvsToml::read(config_path) {
                *config = c;
            }
            Ok(())
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                elapsed_ms(write_start),
            ));
            Err(())
        }
    }
}

/// Read the index metadata and stats, if a complete index exists.
///
/// Infallible: a missing or unreadable index is reported as a completed
/// step with `exists: false` and yields `None`.
pub(crate) async fn read_index_step(
    backend: &Backend,
    steps: &mut Vec<StepEntry>,
) -> Option<(BuildMetadata, IndexStats)> {
    let index_start = Instant::now();
    let index_data = if backend.exists() {
        let build_meta = backend.read_metadata().await.ok().flatten();
        let idx_stats = backend.stats().await.ok().flatten();
        build_meta.zip(idx_stats)
    } else {
        None
    };
    let outcome = match &index_data {
        Some((_, stats)) => ReadIndexOutcome {
            exists: true,
            files_indexed: stats.files_indexed,
            chunks: stats.chunks,
        },
        None => ReadIndexOutcome {
            exists: false,
            files_indexed: 0,
            chunks: 0,
        },
    };
    steps.push(StepEntry::ok(
        Outcome::ReadIndex(outcome),
        elapsed_ms(index_start),
    ));
    index_data
}
