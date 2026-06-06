//! `validate` — the core validation function shared by `mdvs check` and
//! the build pipeline.
//!
//! Orchestrates four passes over the scanned corpus:
//! 1. [`check_frontmatter_errors`] — surface scan-time YAML→JSON
//!    conversion failures as `FrontmatterUnrepresentable` violations.
//! 2. [`check_field_values`] — per-(field, file) jsonschema validation,
//!    plus the path-scoping `Disallowed` check and the strict-Float
//!    precheck, plus new-field discovery on each frontmatter leaf.
//! 3. [`check_required_fields`] — per-field presence check against
//!    `required` globs.
//! 4. [`super::collect::collect_violations`] — sort the accumulator into
//!    the byte-stable `Vec<FieldViolation>` `mdvs check` emits.

use super::CheckResult;
use super::collect::{MappedViolation, ViolationKey, collect_violations, map_validation_error};
use super::field_meta::{FieldMeta, FieldValidators, build_field_metas};
use crate::discover::scan::ScannedFiles;
use crate::output::{NewField, ViolatingFile, ViolationKind};
use crate::preprocess::Pipeline;
use crate::schema::config::MdvsToml;
use jsonschema::error::ValidationErrorKind;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use tracing::{info, instrument};

/// Sentinel field name for document-level violations (e.g.
/// `FrontmatterUnrepresentable`). Sorts before alphabetic field names so
/// these errors appear at the top of the output.
const FRONTMATTER_FIELD_SENTINEL: &str = "<frontmatter>";

/// Validate scanned files against the schema in `mdvs.toml`. Reusable core
/// called by both `mdvs check` (its `run`) and the build pipeline.
#[instrument(name = "validate", skip_all)]
pub fn validate(
    scanned: &ScannedFiles,
    config: &MdvsToml,
    verbose: bool,
) -> anyhow::Result<CheckResult> {
    info!(files = scanned.files.len(), "validating frontmatter");

    let field_map: HashMap<&str, _> = config
        .fields
        .field
        .iter()
        .map(|f| (f.name.as_str(), f))
        .collect();
    let ignore_set: HashSet<&str> = config.fields.ignore.iter().map(|s| s.as_str()).collect();
    let validators = FieldValidators::build(config)?;
    let pipeline = Pipeline::for_config(config);
    // Per-field precomputed metadata (compiled GlobSets for allowed/required,
    // FieldType conversion) so the inner (field, file) loop avoids tens of
    // thousands of redundant `Glob::new`/`FieldType::try_from` calls.
    let field_metas = build_field_metas(config);
    // Per-file path strings, precomputed so `display().to_string()` doesn't
    // run inside the inner loop of `check_required_fields`.
    let file_paths: Vec<String> = scanned
        .files
        .iter()
        .map(|f| f.path.display().to_string())
        .collect();

    let mut violations: HashMap<ViolationKey, Vec<ViolatingFile>> = HashMap::new();
    let mut new_field_paths: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    check_frontmatter_errors(scanned, &mut violations);
    check_field_values(
        scanned,
        &file_paths,
        &field_map,
        &field_metas,
        &ignore_set,
        &validators,
        &pipeline,
        &mut violations,
        &mut new_field_paths,
    );
    check_required_fields(scanned, &file_paths, config, &field_metas, &mut violations);

    let field_violations = collect_violations(violations);
    let new_fields = collect_new_fields(new_field_paths, verbose);

    info!(violations = field_violations.len(), "validation complete");

    Ok(CheckResult {
        files_checked: scanned.files.len(),
        field_violations,
        new_fields,
    })
}

/// Surface scan-time YAML→JSON conversion failures as violations. Files
/// with `frontmatter_error: Some(_)` had broken or unrepresentable
/// frontmatter (NaN/inf, non-string keys, top-level non-object).
fn check_frontmatter_errors(
    scanned: &ScannedFiles,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
) {
    for file in &scanned.files {
        if let Some(reason) = &file.frontmatter_error {
            let key = ViolationKey {
                field: FRONTMATTER_FIELD_SENTINEL.to_string(),
                kind: ViolationKind::FrontmatterUnrepresentable,
                rule: "frontmatter must be a JSON-representable key-value map".to_string(),
            };
            violations.entry(key).or_default().push(ViolatingFile {
                path: file.path.clone(),
                detail: Some(reason.clone()),
            });
        }
    }
}

/// Check each file's frontmatter fields for type mismatches, disallowed locations, and new fields.
///
/// Path-scoping (`allowed` glob) and new-field discovery stay Rust-side because
/// JSON Schema doesn't model per-file location semantics. Per-value checks
/// (type, null-vs-nullable, enum, range, length, pattern, array bounds) are
/// delegated to a per-field `jsonschema::Validator`. Stage-2 preprocessors
/// (configured per-field) run before validation.
#[allow(clippy::too_many_arguments)]
fn check_field_values(
    scanned: &ScannedFiles,
    file_paths: &[String],
    field_map: &HashMap<&str, &crate::schema::config::TomlField>,
    field_metas: &HashMap<String, FieldMeta>,
    ignore_set: &HashSet<&str>,
    validators: &FieldValidators,
    pipeline: &Pipeline,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
    new_field_paths: &mut BTreeMap<String, Vec<PathBuf>>,
) {
    for (file_idx, file) in scanned.files.iter().enumerate() {
        let file_path_str = file_paths[file_idx].as_str();

        let Some(frontmatter) = file.data.as_ref() else {
            continue;
        };
        if !frontmatter.is_object() {
            continue;
        }

        // ---- Per-declared-field pass: navigate by dotted path, validate ----
        //
        // We iterate `[[fields.field]]` entries (not frontmatter keys) because
        // field names may be dotted (TODO-0097 step 1+) and refer to nested
        // leaves. `navigate_dotted` walks the YAML's nested Object structure
        // to retrieve the leaf value.
        for (field_name, toml_field) in field_map {
            let Some(value) = navigate_dotted(frontmatter, field_name) else {
                // Absent — handled by `check_required_fields`.
                continue;
            };

            let meta = field_metas.get(*field_name);

            // Disallowed: field present at a path not in allowed.
            // Use the precompiled GlobSet.
            if let Some(m) = meta
                && !m.allowed.is_match(file_path_str)
            {
                let key = ViolationKey {
                    field: (*field_name).to_string(),
                    kind: ViolationKind::Disallowed,
                    rule: format!("allowed in {:?}", toml_field.allowed),
                };
                violations.entry(key).or_default().push(ViolatingFile {
                    path: file.path.clone(),
                    detail: None,
                });
            }

            // Strict subtype precheck — jsonschema can't see the
            // serde i64/f64 distinction, so we enforce strict-Float
            // (reject integers unless widen_int_to_float is opted in)
            // here in Rust, before the preprocessor + jsonschema.
            // See preprocess::strict_subtype_check for the full rule.
            if let Some(ft) = meta.and_then(|m| m.field_type.as_ref())
                && let Some(detail) = crate::preprocess::strict_subtype_check(toml_field, ft, value)
            {
                let key = ViolationKey {
                    field: (*field_name).to_string(),
                    kind: ViolationKind::WrongType,
                    rule: format!("type {}", toml_field.field_type),
                };
                violations.entry(key).or_default().push(ViolatingFile {
                    path: file.path.clone(),
                    detail: Some(detail),
                });
                continue;
            }

            // Per-value validation via jsonschema (type, null, enum,
            // range, length, pattern, array bounds).
            //
            // Run the Stage-2 preprocessor pipeline first; the
            // resulting value is what jsonschema validates against.
            let preprocessed = pipeline.apply_to_value(toml_field, value);
            let validation_value = preprocessed.as_ref();

            if let Some(validator) = validators.get(field_name) {
                // Fast path: most (field, file) pairs are clean. Skip the
                // error-collection allocation when the value is valid.
                if validator.is_valid(validation_value) {
                    continue;
                }
                let errors: Vec<_> = validator.iter_errors(validation_value).collect();
                let has_type_error = errors
                    .iter()
                    .any(|e| matches!(e.kind(), ValidationErrorKind::Type { .. }));
                for err in &errors {
                    if has_type_error && !matches!(err.kind(), ValidationErrorKind::Type { .. }) {
                        continue;
                    }
                    let mapped: MappedViolation =
                        map_validation_error(err, validation_value, toml_field);
                    let key = ViolationKey {
                        field: (*field_name).to_string(),
                        kind: mapped.kind,
                        rule: mapped.rule,
                    };
                    violations.entry(key).or_default().push(ViolatingFile {
                        path: file.path.clone(),
                        detail: mapped.detail,
                    });
                }
            }
        }

        // ---- New-field detection pass: walk frontmatter leaves ----
        //
        // After TODO-0097 step 1, undeclared fields can be at any depth.
        // Walk the frontmatter's leaf paths (via collect_leaves), reporting
        // any not in `field_map` or `ignore_set` as new fields.
        let mut leaves: Vec<(String, &Value)> = Vec::new();
        crate::discover::infer::collect_leaves(frontmatter, &mut leaves);
        for (leaf_path, _) in leaves {
            if ignore_set.contains(leaf_path.as_str()) {
                continue;
            }
            if field_map.contains_key(leaf_path.as_str()) {
                continue;
            }
            new_field_paths
                .entry(leaf_path)
                .or_default()
                .push(file.path.clone());
        }
    }
}

/// Check that required fields are present in files matching their required glob patterns.
///
/// Leaf presence is determined by [`navigate_dotted`]: a dotted-name field
/// is "present" only if every intermediate object exists. So a file whose
/// frontmatter has `meta: null` (or no `meta` at all) reports `meta.author`
/// as missing — the leaf can't exist when its parent doesn't.
fn check_required_fields(
    scanned: &ScannedFiles,
    file_paths: &[String],
    config: &MdvsToml,
    field_metas: &HashMap<String, FieldMeta>,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
) {
    for toml_field in &config.fields.field {
        if toml_field.required.is_empty() {
            continue;
        }
        let Some(meta) = field_metas.get(toml_field.name.as_str()) else {
            continue;
        };

        for (file_idx, file) in scanned.files.iter().enumerate() {
            if !meta.required.is_match(file_paths[file_idx].as_str()) {
                continue;
            }

            let present = file
                .data
                .as_ref()
                .and_then(|root| navigate_dotted(root, &toml_field.name))
                .is_some();

            // Null on non-nullable is caught by check_field_values — only check absence here
            if !present {
                let key = ViolationKey {
                    field: toml_field.name.clone(),
                    kind: ViolationKind::MissingRequired,
                    rule: format!("required in {:?}", toml_field.required),
                };
                violations.entry(key).or_default().push(ViolatingFile {
                    path: file.path.clone(),
                    detail: None,
                });
            }
        }
    }
}

/// Convert the new fields accumulator into a list of `NewField`.
fn collect_new_fields(
    new_field_paths: BTreeMap<String, Vec<PathBuf>>,
    verbose: bool,
) -> Vec<NewField> {
    new_field_paths
        .into_iter()
        .map(|(name, paths)| {
            let files_found = paths.len();
            NewField {
                name,
                files_found,
                files: if verbose { Some(paths) } else { None },
            }
        })
        .collect()
}

/// Navigate a nested frontmatter `Value` by a dotted path. Returns `None` if
/// any intermediate is missing or is not an Object — so a leaf nested inside
/// an absent parent reads as absent (handled by [`check_required_fields`]).
fn navigate_dotted<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = root;
    for segment in path.split('.') {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}
