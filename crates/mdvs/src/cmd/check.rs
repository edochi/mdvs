use crate::discover::field_type::FieldType;
use crate::discover::infer::InferredSchema;
use crate::discover::scan::ScannedFiles;
use crate::outcome::commands::CheckOutcome;
use crate::outcome::{
    InferOutcome, Outcome, ReadConfigOutcome, ScanOutcome, ValidateOutcome, WriteConfigOutcome,
};
use crate::output::{FieldViolation, NewField, ViolatingFile, ViolationKind};
use crate::preprocess::Pipeline;
use crate::schema::config::{MdvsToml, TomlField};
use crate::schema::json_schema::{canonical_to_dsl, dsl_to_canonical, validate_mdvs_schema};
use crate::schema::load::load_schema;
use crate::schema::shared::FieldTypeSerde;
use crate::step::{CommandResult, ErrorKind, StepEntry};
use globset::Glob;
use jsonschema::error::ValidationErrorKind;
use jsonschema::{ValidationError, Validator};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{info, instrument};

// ============================================================================
// CheckResult — kept for build compatibility during migration
// ============================================================================

/// Result of validation. Used by both `check` and `build` commands.
/// Kept during migration; build still references this type.
#[derive(Debug, Serialize)]
pub struct CheckResult {
    /// Number of markdown files checked.
    pub files_checked: usize,
    /// Schema violations (wrong type, missing required, disallowed).
    pub field_violations: Vec<FieldViolation>,
    /// Fields found in frontmatter but not defined in `mdvs.toml`.
    pub new_fields: Vec<NewField>,
}

impl CheckResult {
    /// Returns `true` if any schema violations were found.
    pub fn has_violations(&self) -> bool {
        !self.field_violations.is_empty()
    }
}

// ============================================================================
// run()
// ============================================================================

/// Read config, optionally auto-update, scan files, and validate frontmatter.
///
/// When `schema_override` is `Some(path)`, the schema file replaces the
/// `mdvs.toml`'s `[fields]` block for this invocation. If no `mdvs.toml`
/// exists, a default config is synthesized around the schema.
/// Auto-update is disabled in either schema-override case (the schema is
/// ephemeral and the toml shouldn't be touched).
#[instrument(name = "check", skip_all)]
pub fn run(
    path: &Path,
    no_update: bool,
    verbose: bool,
    schema_override: Option<&Path>,
) -> CommandResult {
    let start = Instant::now();
    let mut steps = Vec::new();

    // 1. Resolve config. Precedence:
    //    - `--schema PATH` provided + mdvs.toml exists → replace fields, keep
    //      other sections from the toml.
    //    - `--schema PATH` provided + no mdvs.toml → synthesize defaults.
    //    - no `--schema` → read mdvs.toml as usual (error if missing).
    let config_start = std::time::Instant::now();
    let config_path_buf = path.join("mdvs.toml");
    let reported_path = match schema_override {
        Some(p) => p.display().to_string(),
        None => config_path_buf.display().to_string(),
    };
    let config = match resolve_check_config(&config_path_buf, schema_override) {
        Ok(cfg) => {
            steps.push(StepEntry::ok(
                Outcome::ReadConfig(ReadConfigOutcome {
                    config_path: reported_path,
                }),
                config_start.elapsed().as_millis() as u64,
            ));
            cfg
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::User,
                e.to_string(),
                config_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    // Force-disable auto-update when --schema is given: the toml shouldn't
    // be edited as a side effect of a schema-driven invocation.
    let no_update = no_update || schema_override.is_some();

    // 2. Scan (once — shared between auto-update and validate)
    let scan_start = Instant::now();
    let scanned = match ScannedFiles::scan(path, &config.scan) {
        Ok(s) => {
            steps.push(StepEntry::ok(
                Outcome::Scan(ScanOutcome {
                    files_found: s.files.len(),
                    glob: config.scan.glob.clone(),
                }),
                scan_start.elapsed().as_millis() as u64,
            ));
            s
        }
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                scan_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed_from_steps(steps, start);
        }
    };

    // 3. Auto-update: infer new fields, write config if changed
    let should_update = !no_update && config.check.as_ref().is_some_and(|c| c.auto_update);
    let config = if should_update {
        let infer_start = Instant::now();
        let schema = InferredSchema::infer(&scanned);
        steps.push(StepEntry::ok(
            Outcome::Infer(InferOutcome {
                fields_inferred: schema.fields.len(),
            }),
            infer_start.elapsed().as_millis() as u64,
        ));
        schema.emit_dropped_warnings();

        // Find truly new fields (not in config, not ignored)
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
            config
        } else {
            let mut config = config;
            config.fields.field.extend(new_toml_fields);
            let write_start = Instant::now();
            match config.write(&config_path_buf) {
                Ok(()) => {
                    steps.push(StepEntry::ok(
                        Outcome::WriteConfig(WriteConfigOutcome {
                            config_path: config_path_buf.display().to_string(),
                            fields_written: config.fields.field.len(),
                        }),
                        write_start.elapsed().as_millis() as u64,
                    ));
                    // Re-read to pick up normalized TOML
                    match MdvsToml::read(&config_path_buf) {
                        Ok(c) => c,
                        Err(_) => config,
                    }
                }
                Err(e) => {
                    steps.push(StepEntry::err(
                        ErrorKind::Application,
                        e.to_string(),
                        write_start.elapsed().as_millis() as u64,
                    ));
                    return CommandResult::failed(
                        steps,
                        ErrorKind::Application,
                        "auto-update failed to write config".into(),
                        start,
                    );
                }
            }
        }
    } else {
        config
    };

    // 4. Validate
    let validate_start = std::time::Instant::now();
    let check_result = match validate(&scanned, &config, verbose) {
        Ok(r) => r,
        Err(e) => {
            steps.push(StepEntry::err(
                ErrorKind::Application,
                e.to_string(),
                validate_start.elapsed().as_millis() as u64,
            ));
            return CommandResult::failed(
                steps,
                ErrorKind::Application,
                "validation failed".into(),
                start,
            );
        }
    };

    // Push validate step
    steps.push(StepEntry::ok(
        Outcome::Validate(ValidateOutcome {
            files_checked: check_result.files_checked,
            violations: check_result.field_violations.clone(),
            new_fields: check_result.new_fields.clone(),
        }),
        validate_start.elapsed().as_millis() as u64,
    ));

    // Build command outcome
    CommandResult {
        steps,
        result: Ok(Outcome::Check(Box::new(CheckOutcome {
            files_checked: check_result.files_checked,
            violations: check_result.field_violations,
            new_fields: check_result.new_fields,
        }))),
        elapsed_ms: start.elapsed().as_millis() as u64,
    }
}

// ============================================================================
// validate() — core validation logic, reused by build
// ============================================================================

/// Accumulator key for grouping violations by field, kind, and rule.
#[derive(PartialEq, Eq, Hash)]
struct ViolationKey {
    field: String,
    kind: ViolationKind,
    rule: String,
}

/// Resolve the `MdvsToml` for a check invocation, honoring an optional
/// `--schema` override.
///
/// Three cases:
/// - `--schema` given + `mdvs.toml` exists: load and gate the schema, then
///   replace the toml's `[fields]` block (fields + ignore) with the schema's.
///   Other sections (`[scan]`, `[check]`, `[update]`, ...) come from the toml.
/// - `--schema` given + no `mdvs.toml`: synthesize a default config around
///   the schema's fields via `MdvsToml::default_with_fields`.
/// - No `--schema`: read `mdvs.toml` as usual; missing toml is a hard error.
fn resolve_check_config(
    config_path: &Path,
    schema_override: Option<&Path>,
) -> anyhow::Result<MdvsToml> {
    let toml_result = MdvsToml::read(config_path);

    if let Some(schema_path) = schema_override {
        let canonical = load_schema(schema_path)?;
        validate_mdvs_schema(&canonical).map_err(|e| {
            anyhow::anyhow!(
                "schema '{}' is not in the mdvs subset: {e}",
                schema_path.display()
            )
        })?;
        let import = canonical_to_dsl(&canonical).map_err(|e| {
            anyhow::anyhow!("cannot import schema '{}': {e}", schema_path.display())
        })?;
        let cfg = match toml_result {
            Ok(mut existing) => {
                existing.fields.field = import.fields;
                existing.fields.ignore = import.ignore;
                existing
            }
            Err(_) => MdvsToml::default_with_fields(import.fields, import.ignore),
        };
        cfg.validate()?;
        Ok(cfg)
    } else {
        let cfg = toml_result?;
        cfg.validate().map_err(|e| {
            anyhow::anyhow!("mdvs.toml is invalid: {e} — fix the file or run 'mdvs init --force'")
        })?;
        Ok(cfg)
    }
}

/// Validate scanned files against the schema in `mdvs.toml`. Reusable core called by both `check` and `build`.
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

    let mut violations: HashMap<ViolationKey, Vec<ViolatingFile>> = HashMap::new();
    let mut new_field_paths: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();

    check_frontmatter_errors(scanned, &mut violations);
    check_field_values(
        scanned,
        &field_map,
        &ignore_set,
        &validators,
        &pipeline,
        &mut violations,
        &mut new_field_paths,
    );
    check_required_fields(scanned, config, &mut violations);

    let field_violations = collect_violations(violations);
    let new_fields = collect_new_fields(new_field_paths, verbose);

    info!(violations = field_violations.len(), "validation complete");

    Ok(CheckResult {
        files_checked: scanned.files.len(),
        field_violations,
        new_fields,
    })
}

/// Sentinel field name for document-level violations (e.g.
/// `FrontmatterUnrepresentable`). Sorts before alphabetic field names so
/// these errors appear at the top of the output.
const FRONTMATTER_FIELD_SENTINEL: &str = "<frontmatter>";

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
fn check_field_values(
    scanned: &ScannedFiles,
    field_map: &HashMap<&str, &crate::schema::config::TomlField>,
    ignore_set: &HashSet<&str>,
    validators: &FieldValidators,
    pipeline: &Pipeline,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
    new_field_paths: &mut BTreeMap<String, Vec<PathBuf>>,
) {
    for file in &scanned.files {
        let file_path_str = file.path.display().to_string();

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

            // Disallowed: field present at a path not in allowed
            if !matches_any_glob(&toml_field.allowed, &file_path_str) {
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
            if let Ok(ft) = crate::discover::field_type::FieldType::try_from(&toml_field.field_type)
                && let Some(detail) =
                    crate::preprocess::strict_subtype_check(toml_field, &ft, value)
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
                let errors: Vec<_> = validator.iter_errors(validation_value).collect();
                let has_type_error = errors
                    .iter()
                    .any(|e| matches!(e.kind(), ValidationErrorKind::Type { .. }));
                for err in &errors {
                    if has_type_error && !matches!(err.kind(), ValidationErrorKind::Type { .. }) {
                        continue;
                    }
                    let mapped = map_validation_error(err, validation_value, toml_field);
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
    config: &MdvsToml,
    violations: &mut HashMap<ViolationKey, Vec<ViolatingFile>>,
) {
    for toml_field in &config.fields.field {
        if toml_field.required.is_empty() {
            continue;
        }

        for file in &scanned.files {
            let file_path_str = file.path.display().to_string();

            if !matches_any_glob(&toml_field.required, &file_path_str) {
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

/// Convert the violations accumulator into a sorted list of `FieldViolation`.
fn collect_violations(
    violations: HashMap<ViolationKey, Vec<ViolatingFile>>,
) -> Vec<FieldViolation> {
    let mut field_violations: Vec<FieldViolation> = violations
        .into_iter()
        .map(|(key, files)| FieldViolation {
            field: key.field,
            kind: key.kind,
            rule: key.rule,
            files,
        })
        .collect();
    field_violations.sort_by(|a, b| a.field.cmp(&b.field));
    field_violations
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

// ============================================================================
// Per-field jsonschema validators
// ============================================================================

/// Compiled per-field `jsonschema::Validator`s, built once per `validate()`
/// call from `dsl_to_canonical(config)`.
///
/// Per-field is preferred over a single global validator because:
/// - Path-scoping (`allowed`/`required`) stays Rust-side; the global root has
///   `additionalProperties: true` and no `required`, so jsonschema would
///   never produce `Required` or `AdditionalProperties` errors anyway.
/// - The field name is known a priori from the dispatch loop, no need to
///   parse `instance_path`.
struct FieldValidators {
    per_field: HashMap<String, Validator>,
}

impl FieldValidators {
    fn build(config: &MdvsToml) -> anyhow::Result<Self> {
        let canonical = dsl_to_canonical(config);
        let leaf_schemas = extract_leaf_schemas(&canonical);

        let mut per_field = HashMap::new();
        for field in &config.fields.field {
            // Skip fields whose subschema is `{}` (always-passes); compiling
            // is wasted work and produces no errors anyway.
            let Some(subschema) = leaf_schemas.get(field.name.as_str()) else {
                continue;
            };
            if subschema.as_object().is_some_and(|o| o.is_empty()) {
                continue;
            }
            // Strip `x-mdvs` before compiling — it's an extension, not a
            // validation keyword. Carrying it would be harmless (jsonschema
            // ignores unknown keywords by default) but keeps schemas tidy
            // for any future debug printing.
            let stripped = strip_x_mdvs(subschema.clone());
            let validator = jsonschema::options()
                .should_validate_formats(true)
                .build(&stripped)
                .map_err(|e| {
                    anyhow::anyhow!("failed to compile schema for '{}': {e}", field.name)
                })?;
            per_field.insert(field.name.clone(), validator);
        }
        Ok(Self { per_field })
    }

    fn get(&self, field_name: &str) -> Option<&Validator> {
        self.per_field.get(field_name)
    }
}

/// Walk the canonical JSON Schema's nested `properties` tree, returning one
/// entry per **leaf** schema keyed by dotted name.
///
/// Intermediate Objects (created by `dsl_to_canonical` for dotted-name
/// flattening — see TODO-0097 step 3) are recursed into, not emitted. The
/// detection uses [`crate::schema::json_schema::is_intermediate_object`]:
/// `{type: "object", properties: {...}}` with no `x-mdvs` is an intermediate;
/// anything else (scalars, arrays, leaf objects with x-mdvs) is a leaf.
fn extract_leaf_schemas(canonical: &Value) -> HashMap<String, Value> {
    let mut out = HashMap::new();
    if let Some(root) = canonical.get("properties").and_then(Value::as_object) {
        walk_schema_leaves(root, "", &mut out);
    }
    out
}

fn walk_schema_leaves(props: &Map<String, Value>, prefix: &str, out: &mut HashMap<String, Value>) {
    for (name, sub) in props {
        let full = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}.{name}")
        };
        if crate::schema::json_schema::is_intermediate_object(sub) {
            if let Some(inner) = sub.get("properties").and_then(Value::as_object) {
                walk_schema_leaves(inner, &full, out);
            }
        } else {
            out.insert(full, sub.clone());
        }
    }
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

fn strip_x_mdvs(mut value: Value) -> Value {
    if let Value::Object(map) = &mut value {
        map.remove("x-mdvs");
        if let Some(Value::Object(props)) = map.get_mut("properties") {
            for v in props.values_mut() {
                let taken = std::mem::take(v);
                *v = strip_x_mdvs(taken);
            }
        }
        if let Some(items) = map.get_mut("items") {
            let taken = std::mem::take(items);
            *items = strip_x_mdvs(taken);
        }
    }
    value
}

// ============================================================================
// ValidationError → ViolationKind mapping
// ============================================================================

struct MappedViolation {
    kind: ViolationKind,
    rule: String,
    detail: Option<String>,
}

/// Translate a `jsonschema::ValidationError` into the mdvs `ViolationKind`
/// shape, using the field's TOML config for rule strings.
///
/// The two non-mechanical cases:
/// - `Type` against a `null` instance → `NullNotAllowed` (not `WrongType`).
/// - `Pattern` mismatch → `WrongType` (no dedicated `PatternMismatch` variant
///   in v0; the rule string carries the pattern).
fn map_validation_error(
    err: &ValidationError,
    value: &Value,
    field: &TomlField,
) -> MappedViolation {
    use ValidationErrorKind as E;

    // Resolve the actual offending instance — for top-level errors it's the
    // value we passed; for `items` errors it's at `instance_path` index N.
    let instance = resolve_instance_path(value, &err.instance_path().to_string());

    match err.kind() {
        E::Type { .. } => {
            if instance.is_null() {
                MappedViolation {
                    kind: ViolationKind::NullNotAllowed,
                    rule: "not nullable".to_string(),
                    detail: None,
                }
            } else {
                MappedViolation {
                    kind: ViolationKind::WrongType,
                    rule: format!("type {}", field.field_type),
                    detail: Some(format!("got {}", actual_type_name(instance))),
                }
            }
        }
        E::Required { property } => MappedViolation {
            kind: ViolationKind::MissingRequired,
            rule: format!("required '{}'", property.as_str().unwrap_or("?")),
            detail: None,
        },
        E::AdditionalProperties { unexpected } => MappedViolation {
            kind: ViolationKind::Disallowed,
            rule: "additionalProperties = false".to_string(),
            detail: Some(format!("unexpected: {unexpected:?}")),
        },
        // For value-comparing errors (enum, const, range, length, pattern,
        // array bounds), the rule string carries the constraint and the
        // detail carries just the offending value as `got <json>`. Avoids
        // duplicating the rule in every detail line.
        E::Enum { options } => MappedViolation {
            kind: ViolationKind::InvalidCategory,
            rule: format!("enum {options}"),
            detail: Some(format!("got {instance}")),
        },
        E::Constant { expected_value } => MappedViolation {
            kind: ViolationKind::InvalidCategory,
            rule: format!("const {expected_value}"),
            detail: Some(format!("got {instance}")),
        },
        E::Minimum { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("minimum {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::Maximum { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("maximum {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::ExclusiveMinimum { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("exclusiveMinimum {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::ExclusiveMaximum { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("exclusiveMaximum {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::MultipleOf { multiple_of } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("multipleOf {multiple_of}"),
            detail: Some(format!("got {instance}")),
        },
        E::MinLength { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("minLength {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::MaxLength { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("maxLength {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::Pattern { pattern } => MappedViolation {
            kind: ViolationKind::WrongType,
            rule: format!("pattern {pattern}"),
            detail: Some(format!("got {instance}")),
        },
        E::Format { format } => MappedViolation {
            kind: ViolationKind::WrongType,
            rule: format!("format {format}"),
            detail: Some(format!("got {instance}")),
        },
        E::MinItems { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("minItems {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::MaxItems { limit } => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: format!("maxItems {limit}"),
            detail: Some(format!("got {instance}")),
        },
        E::UniqueItems => MappedViolation {
            kind: ViolationKind::OutOfRange,
            rule: "uniqueItems".to_string(),
            detail: Some(format!("got {instance}")),
        },
        // Variants below should be unreachable in practice: validate_mdvs_schema
        // (the gate) rejects every schema that could trigger them upstream.
        // We bucket them defensively so the binary doesn't panic — bug reports
        // with these messages indicate a gate hole.
        E::AdditionalItems { .. }
        | E::AnyOf { .. }
        | E::BacktrackLimitExceeded { .. }
        | E::RegexEngineFailure { .. }
        | E::Contains
        | E::ContentEncoding { .. }
        | E::ContentMediaType { .. }
        | E::Custom { .. }
        | E::FalseSchema
        | E::FromUtf8 { .. }
        | E::MaxProperties { .. }
        | E::MinProperties { .. }
        | E::Not { .. }
        | E::OneOfMultipleValid { .. }
        | E::OneOfNotValid { .. }
        | E::PropertyNames { .. }
        | E::UnevaluatedItems { .. }
        | E::UnevaluatedProperties { .. }
        | E::Referencing(_) => MappedViolation {
            kind: ViolationKind::WrongType,
            rule: format!(
                "unexpected validator error ({}) — schema gate should reject this; please report",
                err.kind().keyword()
            ),
            detail: Some(err.to_string()),
        },
    }
}

/// Resolve a JSON Pointer (e.g. `/0` or `/items/1`) relative to `root`.
/// Used to find the offending sub-value when an error fires inside `items`.
fn resolve_instance_path<'a>(root: &'a Value, path: &str) -> &'a Value {
    let mut cur = root;
    for seg in path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
    {
        cur = match cur {
            Value::Object(m) => m.get(seg).unwrap_or(&Value::Null),
            Value::Array(a) => a
                .get(seg.parse::<usize>().unwrap_or(0))
                .unwrap_or(&Value::Null),
            _ => &Value::Null,
        };
    }
    cur
}

fn matches_any_glob(patterns: &[String], path: &str) -> bool {
    patterns.iter().any(|p| {
        Glob::new(p)
            .ok()
            .map(|g| g.compile_matcher())
            .is_some_and(|m| m.is_match(path))
    })
}

fn actual_type_name(value: &Value) -> String {
    FieldTypeSerde::from(&FieldType::from(value)).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome::commands::CheckOutcome;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::ScanConfig;
    use std::fs;

    fn unwrap_check(result: &CommandResult) -> &CheckOutcome {
        match &result.result {
            Ok(Outcome::Check(o)) => o,
            other => panic!("expected Ok(Check), got: {other:?}"),
        }
    }

    fn create_test_vault(dir: &Path) {
        let blog_dir = dir.join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ntags:\n  - rust\n  - code\ndraft: false\n---\n# Hello\nBody text.",
        )
        .unwrap();

        fs::write(
            blog_dir.join("post2.md"),
            "---\ntitle: World\ndraft: true\n---\n# World\nMore text.",
        )
        .unwrap();
    }

    fn write_toml(dir: &Path, fields: Vec<TomlField>, ignore: Vec<String>) {
        let mut config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore,
                field: fields,
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        config.write(&dir.join("mdvs.toml")).unwrap();
    }

    fn string_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    fn bool_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("Boolean".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    #[test]
    fn clean_check() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());
        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert!(result.new_fields.is_empty());
        assert_eq!(result.files_checked, 2);
    }

    #[test]
    fn missing_required() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
                bool_field("draft"),
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "tags");
        assert!(matches!(v.kind, ViolationKind::MissingRequired));
        assert_eq!(v.files.len(), 1);
        assert_eq!(v.files[0].path.display().to_string(), "blog/post2.md");
    }

    #[test]
    fn wrong_type() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        fs::create_dir_all(&blog_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![string_field("title"), bool_field("draft")],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got String"));
    }

    #[test]
    fn wrong_type_int_in_float_strict() {
        // Strict-Float: integer values are rejected unless widen_int_to_float
        // is in preprocess. See preprocess::strict_subtype_check.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nrating: 5\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "rating".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        let v = &result.violations[0];
        assert_eq!(v.field, "rating");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got Integer"));
    }

    #[test]
    fn int_in_float_with_widen_int_to_float_passes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nrating: 5\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "rating".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![crate::preprocess::ValueStage::WidenIntToFloat],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn float_value_in_strict_float_passes() {
        // Regression guard: pure float `5.0` must not be flagged by the
        // strict precheck (it targets i64-backed numbers only).
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nrating: 5.0\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "rating".into(),
                field_type: FieldTypeSerde::Scalar("Float".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn int_in_array_float_strict() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nscores: [1.0, 2, 3.0]\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "scores".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("Float".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        let v = &result.violations[0];
        assert_eq!(v.field, "scores");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got Integer at index 1"));
    }

    #[test]
    fn int_in_array_float_with_widen_passes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/post1.md"),
            "---\nscores: [1.0, 2, 3.0]\n---\n# Post\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "scores".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("Float".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![crate::preprocess::ValueStage::WidenIntToFloat],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn disallowed_field() {
        let tmp = tempfile::tempdir().unwrap();
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&notes_dir).unwrap();

        fs::write(
            notes_dir.join("idea.md"),
            "---\ndraft: true\n---\n# Idea\nContent.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "draft".into(),
                field_type: FieldTypeSerde::Scalar("Boolean".into()),
                allowed: vec!["blog/**".into()],
                required: vec![],
                nullable: false,
                constraints: None,
                preprocess: vec![],
            }],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        let v = &result.violations[0];
        assert_eq!(v.field, "draft");
        assert!(matches!(v.kind, ViolationKind::Disallowed));
        assert_eq!(v.files[0].path.display().to_string(), "notes/idea.md");
    }

    #[test]
    fn new_fields_informational() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        write_toml(tmp.path(), vec![string_field("title")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert_eq!(result.new_fields.len(), 2);
        assert!(result.new_fields.iter().any(|f| f.name == "draft"));
        assert!(result.new_fields.iter().any(|f| f.name == "tags"));
    }

    // Note: the legacy "String is top type" test that exercised hand-written
    // toml accepting bool/int/array/object on a String field was deleted with
    // TODO-0149 step 3. Coercion is now an explicit `preprocess` choice
    // recorded in the toml, not an implicit runtime hack.

    #[test]
    fn bare_files_trigger_required() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();

        fs::write(
            tmp.path().join("blog/bare.md"),
            "# No frontmatter\nJust content.",
        )
        .unwrap();

        let mut config = MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: true,
                skip_gitignore: false,
            },
            update: UpdateConfig {},
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                }],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        };
        config.write(&tmp.path().join("mdvs.toml")).unwrap();

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
    }

    #[test]
    fn ignored_fields_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        create_test_vault(tmp.path());

        write_toml(
            tmp.path(),
            vec![string_field("title")],
            vec!["draft".into(), "tags".into()],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(result.violations.is_empty());
        assert!(result.new_fields.is_empty());
    }

    #[test]
    fn multiple_violations() {
        let tmp = tempfile::tempdir().unwrap();
        let blog_dir = tmp.path().join("blog");
        let notes_dir = tmp.path().join("notes");
        fs::create_dir_all(&blog_dir).unwrap();
        fs::create_dir_all(&notes_dir).unwrap();

        fs::write(
            blog_dir.join("post1.md"),
            "---\ntitle: Hello\ndraft: \"yes\"\n---\n# Post\nBody.",
        )
        .unwrap();

        fs::write(
            notes_dir.join("note1.md"),
            "---\ntitle: Note\ndraft: true\n---\n# Note\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
                TomlField {
                    name: "tags".into(),
                    field_type: FieldTypeSerde::Array {
                        array: Box::new(FieldTypeSerde::Scalar("String".into())),
                    },
                    allowed: vec!["**".into()],
                    required: vec!["blog/**".into()],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        assert!(!result.violations.is_empty());
        assert!(result.violations.len() >= 3);
    }

    #[test]
    fn null_on_non_nullable_non_required_field() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\nstatus:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "status".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "status")
            .expect("expected NullNotAllowed for status");
        assert!(matches!(v.kind, ViolationKind::NullNotAllowed));
    }

    #[test]
    fn null_on_nullable_non_required_field() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\nstatus:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "status".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: true,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn null_on_disallowed_path() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\ndraft:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                    nullable: true,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "draft")
            .expect("expected Disallowed for draft");
        assert!(matches!(v.kind, ViolationKind::Disallowed));
    }

    #[test]
    fn null_on_disallowed_path_and_not_nullable() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\ndraft:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "draft".into(),
                    field_type: FieldTypeSerde::Scalar("Boolean".into()),
                    allowed: vec!["blog/**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(!result.violations.is_empty());

        let has_disallowed = result
            .violations
            .iter()
            .any(|v| v.field == "draft" && matches!(v.kind, ViolationKind::Disallowed));
        let has_null_not_allowed = result
            .violations
            .iter()
            .any(|v| v.field == "draft" && matches!(v.kind, ViolationKind::NullNotAllowed));

        assert!(has_disallowed, "expected Disallowed for draft");
        assert!(has_null_not_allowed, "expected NullNotAllowed for draft");
    }

    #[test]
    fn null_on_required_non_nullable_produces_single_violation() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();

        fs::write(
            tmp.path().join("notes/note1.md"),
            "---\ntitle: Hello\nstatus:\n---\n# Hello\nBody.",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![
                string_field("title"),
                TomlField {
                    name: "status".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec!["**".into()],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                },
            ],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);

        // Should produce exactly 1 NullNotAllowed — not duplicated by check_required_fields
        let null_violations: Vec<_> = result
            .violations
            .iter()
            .filter(|v| v.field == "status" && matches!(v.kind, ViolationKind::NullNotAllowed))
            .collect();
        assert_eq!(
            null_violations.len(),
            1,
            "expected exactly 1 NullNotAllowed, got {}",
            null_violations.len()
        );
    }

    // --- Unit tests for matches_any_glob ---

    #[test]
    fn glob_star_star_matches_all() {
        assert!(matches_any_glob(&["**".into()], "blog/post.md"));
        assert!(matches_any_glob(&["**".into()], "deep/nested/path.md"));
    }

    #[test]
    fn glob_specific_path() {
        assert!(matches_any_glob(&["blog/**".into()], "blog/post.md"));
        assert!(!matches_any_glob(&["blog/**".into()], "notes/idea.md"));
    }

    #[test]
    fn glob_empty_patterns() {
        assert!(!matches_any_glob(&[], "anything.md"));
    }

    // -----------------------------------------------------------------------
    // Constraint validation (InvalidCategory)
    // -----------------------------------------------------------------------

    fn categorical_field(name: &str, categories: Vec<&str>) -> TomlField {
        use crate::schema::constraints::Constraints;
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("String".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: Some(Constraints {
                categories: Some(
                    categories
                        .into_iter()
                        .map(|s| toml::Value::String(s.into()))
                        .collect(),
                ),
                ..Default::default()
            }),
            preprocess: vec![],
        }
    }

    #[test]
    fn invalid_category_string_detected() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\nstatus: pending\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![categorical_field("status", vec!["draft", "published"])],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::InvalidCategory);
        assert!(
            result.violations[0].files[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("\"pending\"")
        );
    }

    #[test]
    fn valid_category_passes() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\nstatus: draft\n---\nBody.").unwrap();
        write_toml(
            tmp.path(),
            vec![categorical_field("status", vec!["draft", "published"])],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn invalid_category_array_element() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\ntags:\n  - rust\n  - java\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "tags".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("String".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    categories: Some(vec![
                        toml::Value::String("rust".into()),
                        toml::Value::String("python".into()),
                        toml::Value::String("go".into()),
                    ]),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::InvalidCategory);
        let detail = result.violations[0].files[0].detail.as_ref().unwrap();
        assert!(detail.contains("\"java\""));
        assert!(!detail.contains("\"rust\""));
    }

    #[test]
    fn null_on_categorical_nullable_passes() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\nstatus:\n---\nBody.").unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "status".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: true,
                constraints: Some(Constraints {
                    categories: Some(vec![
                        toml::Value::String("draft".into()),
                        toml::Value::String("published".into()),
                    ]),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn wrong_type_on_categorical_no_double_violation() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\ncount: not_a_number\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "count".into(),
                field_type: FieldTypeSerde::Scalar("Integer".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    categories: Some(vec![
                        toml::Value::Integer(1),
                        toml::Value::Integer(2),
                        toml::Value::Integer(3),
                    ]),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        // Only WrongType, not InvalidCategory (constraints skipped when type fails)
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::WrongType);
    }

    // -----------------------------------------------------------------------
    // Integration: init → check with categories
    // -----------------------------------------------------------------------

    #[test]
    fn init_then_check_with_categories_passes() {
        let tmp = tempfile::tempdir().unwrap();
        let blog = tmp.path().join("blog");
        fs::create_dir_all(&blog).unwrap();
        for (i, status) in [
            "draft",
            "draft",
            "published",
            "published",
            "archived",
            "archived",
        ]
        .iter()
        .enumerate()
        {
            fs::write(
                blog.join(format!("post{i}.md")),
                format!("---\nstatus: {status}\ntitle: Post {i}\n---\nBody."),
            )
            .unwrap();
        }

        // Init infers categories on status
        let init_step =
            crate::cmd::init::run(tmp.path(), "**", false, false, true, false, false, None);
        assert!(!crate::step::has_failed(&init_step));

        // Check should pass — inferred categories match actual data
        let check_step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&check_step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn init_then_check_mixed_string_array_no_violations() {
        // Reproducer for TODO-0151: funding is "internal" in some files,
        // ["internal"] in others. Type widens to String. init should infer
        // categories that include both forms. check must not produce violations.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("notes");
        fs::create_dir_all(&dir).unwrap();

        for i in 0..4 {
            fs::write(
                dir.join(format!("a{i}.md")),
                format!("---\nfunding: internal\ntitle: A{i}\n---\nBody."),
            )
            .unwrap();
        }
        for i in 0..3 {
            fs::write(
                dir.join(format!("b{i}.md")),
                format!("---\nfunding:\n  - internal\ntitle: B{i}\n---\nBody."),
            )
            .unwrap();
        }

        let init_step =
            crate::cmd::init::run(tmp.path(), "**", false, false, true, false, false, None);
        assert!(!crate::step::has_failed(&init_step));

        let check_step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&check_step);
        assert!(
            result.violations.is_empty(),
            "expected no violations after init, got: {:?}",
            result.violations
        );
    }

    #[test]
    fn init_then_corrupt_then_check_catches_invalid_category() {
        let tmp = tempfile::tempdir().unwrap();
        let blog = tmp.path().join("blog");
        fs::create_dir_all(&blog).unwrap();
        for (i, status) in [
            "draft",
            "draft",
            "draft",
            "published",
            "published",
            "published",
            "archived",
            "archived",
            "archived",
        ]
        .iter()
        .enumerate()
        {
            fs::write(
                blog.join(format!("post{i}.md")),
                format!("---\nstatus: {status}\ntitle: Post {i}\n---\nBody."),
            )
            .unwrap();
        }

        // Init infers categories
        let init_step =
            crate::cmd::init::run(tmp.path(), "**", false, false, true, false, false, None);
        assert!(!crate::step::has_failed(&init_step));

        // Corrupt a file with an out-of-category value
        fs::write(
            blog.join("post0.md"),
            "---\nstatus: pending\ntitle: Post 0\n---\nBody.",
        )
        .unwrap();

        // Check should catch InvalidCategory
        let check_step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&check_step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::InvalidCategory);
        assert!(
            result.violations[0].files[0]
                .detail
                .as_ref()
                .unwrap()
                .contains("\"pending\"")
        );
    }

    // ========================================================================
    // Frontmatter unrepresentable (TODO-0149 step 8)
    // ========================================================================

    #[test]
    fn check_reports_frontmatter_unrepresentable_for_top_level_array() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("bad.md"), "---\n- a\n- b\n---\nBody.").unwrap();
        write_toml(tmp.path(), vec![string_field("title")], vec![]);
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| matches!(v.kind, ViolationKind::FrontmatterUnrepresentable))
            .expect("expected FrontmatterUnrepresentable violation");
        assert_eq!(v.field, "<frontmatter>");
        let detail = v.files[0].detail.as_ref().unwrap();
        assert!(detail.contains("array"), "got: {detail}");
    }

    #[test]
    fn check_reports_frontmatter_unrepresentable_for_top_level_scalar() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("scalar.md"), "---\nhello\n---\nBody.").unwrap();
        write_toml(tmp.path(), vec![string_field("title")], vec![]);
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(
            result
                .violations
                .iter()
                .any(|v| matches!(v.kind, ViolationKind::FrontmatterUnrepresentable))
        );
    }

    #[test]
    fn check_passes_for_valid_frontmatter() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("ok.md"), "---\ntitle: Hi\n---\nBody.").unwrap();
        write_toml(tmp.path(), vec![string_field("title")], vec![]);
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(
            !result
                .violations
                .iter()
                .any(|v| matches!(v.kind, ViolationKind::FrontmatterUnrepresentable))
        );
    }

    // ========================================================================
    // --schema override (TODO-0149 step 10) — end-to-end
    // ========================================================================

    fn write_schema(dir: &Path, content: &str) -> std::path::PathBuf {
        let path = dir.join("schema.json");
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn check_with_schema_uses_schema_fields() {
        // mdvs.toml exists but the schema replaces its [fields] block.
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: hi\n---\nBody.").unwrap();
        write_toml(tmp.path(), vec![string_field("unrelated")], vec![]);
        let schema_path = write_schema(
            tmp.path(),
            r#"{
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 10}
                },
                "additionalProperties": true
            }"#,
        );
        let step = run(tmp.path(), true, false, Some(&schema_path));
        let result = unwrap_check(&step);
        // The schema's minLength=10 catches "hi" (len 2) — and the toml's
        // "unrelated" field is gone, so no spurious "missing required" etc.
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::OutOfRange);
    }

    #[test]
    fn check_with_schema_works_without_mdvs_toml() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: Hello\n---\nBody.").unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{
                "type": "object",
                "properties": {
                    "title": {"type": "string", "minLength": 3}
                },
                "additionalProperties": true
            }"#,
        );
        let step = run(tmp.path(), true, false, Some(&schema_path));
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn check_with_invalid_schema_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: Hi\n---\nBody.").unwrap();
        let schema_path = write_schema(
            tmp.path(),
            r#"{"oneOf": [{"type": "string"}, {"type": "integer"}]}"#,
        );
        let step = run(tmp.path(), true, false, Some(&schema_path));
        // gate rejects oneOf → command fails before validating
        assert!(crate::step::has_failed(&step));
    }

    #[test]
    fn check_no_toml_no_schema_errors() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: Hi\n---\nBody.").unwrap();
        let step = run(tmp.path(), true, false, None);
        assert!(crate::step::has_failed(&step));
    }

    // ========================================================================
    // Length / pattern constraints (TODO-0149 step 7) — end-to-end
    // ========================================================================

    #[test]
    fn check_min_length_violation() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: ab\n---\nBody.").unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "title".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    min_length: Some(3),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::OutOfRange);
        assert!(result.violations[0].rule.contains("minLength"));
    }

    #[test]
    fn check_max_length_violation() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\ntitle: This title is way too long\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "title".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    max_length: Some(10),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::OutOfRange);
        assert!(result.violations[0].rule.contains("maxLength"));
    }

    #[test]
    fn check_pattern_violation() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\nslug: HasUppercase\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "slug".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    pattern: Some("^[a-z0-9-]+$".into()),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        // Pattern mismatches map to WrongType (per the spike).
        assert_eq!(result.violations[0].kind, ViolationKind::WrongType);
        assert!(result.violations[0].rule.contains("pattern"));
    }

    #[test]
    fn check_length_passes_within_bounds() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("post.md"), "---\ntitle: hello\n---\nBody.").unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "title".into(),
                field_type: FieldTypeSerde::Scalar("String".into()),
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    min_length: Some(3),
                    max_length: Some(10),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(result.violations.is_empty());
    }

    #[test]
    fn check_pattern_array_element_violation() {
        use crate::schema::constraints::Constraints;
        let tmp = tempfile::tempdir().unwrap();
        fs::write(
            tmp.path().join("post.md"),
            "---\ntags:\n  - rust\n  - BAD\n---\nBody.",
        )
        .unwrap();
        write_toml(
            tmp.path(),
            vec![TomlField {
                name: "tags".into(),
                field_type: FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar("String".into())),
                },
                allowed: vec!["**".into()],
                required: vec![],
                nullable: false,
                constraints: Some(Constraints {
                    pattern: Some("^[a-z]+$".into()),
                    ..Default::default()
                }),
                preprocess: vec![],
            }],
            vec![],
        );
        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        assert_eq!(result.violations[0].kind, ViolationKind::WrongType);
    }

    // ========================================================================
    // ValidationError → ViolationKind mapping tests (22 cases lifted from
    // scripts/test_violation_mapping.rs). Each case compiles a minimal schema,
    // runs the validator against the offending instance, and asserts the
    // mapped ViolationKind via map_validation_error.
    // ========================================================================

    fn dummy_field(field_type: FieldTypeSerde) -> TomlField {
        TomlField {
            name: "f".into(),
            field_type,
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    /// Run schema against instance, map all errors, return the resulting kinds.
    fn mapped_kinds(schema: serde_json::Value, instance: serde_json::Value) -> Vec<ViolationKind> {
        let validator = jsonschema::validator_for(&schema).expect("schema compiles");
        let f = dummy_field(FieldTypeSerde::Scalar("String".into()));
        validator
            .iter_errors(&instance)
            .map(|err| map_validation_error(&err, &instance, &f).kind)
            .collect()
    }

    #[test]
    fn map_required_to_missing_required() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "object", "required": ["x"], "properties": {"x": {"type": "string"}}}),
            serde_json::json!({}),
        );
        assert_eq!(kinds, vec![ViolationKind::MissingRequired]);
    }

    #[test]
    fn map_additional_properties_to_disallowed() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "object", "properties": {}, "additionalProperties": false}),
            serde_json::json!({"rogue": 1}),
        );
        assert_eq!(kinds, vec![ViolationKind::Disallowed]);
    }

    #[test]
    fn map_type_string_got_integer_to_wrong_type() {
        let kinds = mapped_kinds(serde_json::json!({"type": "string"}), serde_json::json!(42));
        assert_eq!(kinds, vec![ViolationKind::WrongType]);
    }

    #[test]
    fn map_type_object_got_array_to_wrong_type() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "object"}),
            serde_json::json!([1, 2]),
        );
        assert_eq!(kinds, vec![ViolationKind::WrongType]);
    }

    #[test]
    fn map_type_string_got_null_to_null_not_allowed() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "string"}),
            serde_json::json!(null),
        );
        assert_eq!(kinds, vec![ViolationKind::NullNotAllowed]);
    }

    #[test]
    fn map_type_union_no_violation_for_null_when_listed() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": ["string", "null"]}),
            serde_json::json!(null),
        );
        assert!(kinds.is_empty());
    }

    #[test]
    fn map_enum_to_invalid_category() {
        let kinds = mapped_kinds(
            serde_json::json!({"enum": ["draft", "published", "archived"]}),
            serde_json::json!("scheduled"),
        );
        assert_eq!(kinds, vec![ViolationKind::InvalidCategory]);
    }

    #[test]
    fn map_const_to_invalid_category() {
        let kinds = mapped_kinds(
            serde_json::json!({"const": "fixed"}),
            serde_json::json!("other"),
        );
        assert_eq!(kinds, vec![ViolationKind::InvalidCategory]);
    }

    #[test]
    fn map_minimum_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "integer", "minimum": 0}),
            serde_json::json!(-1),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_maximum_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "integer", "maximum": 100}),
            serde_json::json!(150),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_exclusive_minimum_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "number", "exclusiveMinimum": 0}),
            serde_json::json!(0),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_exclusive_maximum_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "number", "exclusiveMaximum": 1}),
            serde_json::json!(1),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_multiple_of_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "integer", "multipleOf": 5}),
            serde_json::json!(7),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_min_length_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "string", "minLength": 3}),
            serde_json::json!("ab"),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_max_length_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "string", "maxLength": 5}),
            serde_json::json!("too long"),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_pattern_to_wrong_type() {
        // Pattern mismatch ≈ value isn't shaped right for the field's purpose.
        let kinds = mapped_kinds(
            serde_json::json!({"type": "string", "pattern": "^[A-Z]+$"}),
            serde_json::json!("lowercase"),
        );
        assert_eq!(kinds, vec![ViolationKind::WrongType]);
    }

    #[test]
    fn map_min_items_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "array", "minItems": 2}),
            serde_json::json!([1]),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_max_items_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "array", "maxItems": 2}),
            serde_json::json!([1, 2, 3]),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_unique_items_to_out_of_range() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "array", "uniqueItems": true}),
            serde_json::json!([1, 2, 2]),
        );
        assert_eq!(kinds, vec![ViolationKind::OutOfRange]);
    }

    #[test]
    fn map_array_item_type_error_to_wrong_type() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "array", "items": {"type": "string"}}),
            serde_json::json!(["ok", 42, "also ok"]),
        );
        assert_eq!(kinds, vec![ViolationKind::WrongType]);
    }

    #[test]
    fn map_nested_property_type_error() {
        let kinds = mapped_kinds(
            serde_json::json!({"type": "object", "properties": {"draft": {"type": "boolean"}}}),
            serde_json::json!({"draft": "yes please"}),
        );
        assert_eq!(kinds, vec![ViolationKind::WrongType]);
    }

    #[test]
    fn map_multiple_violations_in_one_document() {
        let kinds = mapped_kinds(
            serde_json::json!({
                "type": "object",
                "required": ["title"],
                "properties": {
                    "title": {"type": "string"},
                    "draft": {"type": "boolean"}
                }
            }),
            serde_json::json!({"draft": "yes please"}),
        );
        // Both Required (missing title) and Type (draft) should be mapped.
        assert!(kinds.contains(&ViolationKind::MissingRequired));
        assert!(kinds.contains(&ViolationKind::WrongType));
    }

    // ========================================================================
    // TODO-0097 step 4: dotted-name leaf validation
    // ========================================================================

    fn dotted_leaf_field(name: &str, ty: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar(ty.into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    #[test]
    fn wrong_type_on_dotted_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\ncalibration:\n  baseline:\n    wavelength: \"not a number\"\n---\n# Body",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![dotted_leaf_field(
                "calibration.baseline.wavelength",
                "Float",
            )],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert_eq!(result.violations.len(), 1);
        let v = &result.violations[0];
        assert_eq!(v.field, "calibration.baseline.wavelength");
        assert!(matches!(v.kind, ViolationKind::WrongType));
        assert_eq!(v.files[0].detail.as_deref(), Some("got String"));
    }

    #[test]
    fn missing_required_on_dotted_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        let notes_dir = tmp.path().join("projects/alpha/notes");
        fs::create_dir_all(&notes_dir).unwrap();
        // No `calibration` key at all in this file.
        fs::write(notes_dir.join("exp.md"), "---\ntitle: \"x\"\n---\n# Body").unwrap();

        let mut field = dotted_leaf_field("calibration.baseline.wavelength", "Float");
        field.required = vec!["projects/alpha/notes/**".into()];
        write_toml(tmp.path(), vec![field], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let missing: Vec<_> = result
            .violations
            .iter()
            .filter(|v| {
                v.field == "calibration.baseline.wavelength"
                    && matches!(v.kind, ViolationKind::MissingRequired)
            })
            .collect();
        assert_eq!(missing.len(), 1);
    }

    #[test]
    fn missing_required_on_dotted_leaf_when_parent_is_present_without_child() {
        // The intermediate `calibration.baseline` exists but lacks `wavelength`.
        let tmp = tempfile::tempdir().unwrap();
        let notes_dir = tmp.path().join("projects/alpha/notes");
        fs::create_dir_all(&notes_dir).unwrap();
        fs::write(
            notes_dir.join("exp.md"),
            "---\ncalibration:\n  baseline:\n    intensity: 0.95\n---\n# Body",
        )
        .unwrap();

        let mut wave = dotted_leaf_field("calibration.baseline.wavelength", "Float");
        wave.required = vec!["projects/alpha/notes/**".into()];
        let intensity = dotted_leaf_field("calibration.baseline.intensity", "Float");
        write_toml(tmp.path(), vec![wave, intensity], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let kinds: Vec<_> = result
            .violations
            .iter()
            .map(|v| (&v.field, &v.kind))
            .collect();
        assert!(kinds.iter().any(|(f, k)| {
            f.as_str() == "calibration.baseline.wavelength"
                && matches!(k, ViolationKind::MissingRequired)
        }));
    }

    #[test]
    fn null_not_allowed_on_dotted_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\ncalibration:\n  baseline:\n    wavelength: null\n---\n# Body",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![dotted_leaf_field(
                "calibration.baseline.wavelength",
                "Float",
            )],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = &result.violations[0];
        assert_eq!(v.field, "calibration.baseline.wavelength");
        assert!(matches!(v.kind, ViolationKind::NullNotAllowed));
    }

    #[test]
    fn disallowed_on_dotted_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("notes")).unwrap();
        fs::write(
            tmp.path().join("notes/idea.md"),
            "---\ncalibration:\n  baseline:\n    wavelength: 850.0\n---\n# Body",
        )
        .unwrap();

        let mut field = dotted_leaf_field("calibration.baseline.wavelength", "Float");
        field.allowed = vec!["projects/alpha/**".into()];
        write_toml(tmp.path(), vec![field], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| {
                v.field == "calibration.baseline.wavelength"
                    && matches!(v.kind, ViolationKind::Disallowed)
            })
            .expect("expected Disallowed for calibration.baseline.wavelength");
        assert!(v.rule.contains("projects/alpha"));
    }

    #[test]
    fn new_dotted_field_reported() {
        // Declared: cal.baseline.wavelength. File has cal.baseline.wavelength
        // AND cal.baseline.extra. The undeclared leaf surfaces as a new field.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\ncal:\n  baseline:\n    wavelength: 850.0\n    extra: 5\n---\n# Body",
        )
        .unwrap();

        write_toml(
            tmp.path(),
            vec![dotted_leaf_field("cal.baseline.wavelength", "Float")],
            vec![],
        );

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let names: Vec<&str> = result.new_fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"cal.baseline.extra"), "names: {names:?}");
        assert!(!names.contains(&"cal.baseline.wavelength"));
    }

    // NOTE: a former `array_of_object_validates_inner_shape` test was removed
    // as part of TODO-0155 (Array(Object{...}) is no longer a valid on-disk
    // type — see `parse_rejects_array_of_object` in schema::shared::tests).

    // ===== Date type (TODO-0007 Wave 1) =====

    fn date_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("Date".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    #[test]
    fn date_field_accepts_rfc3339_date() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nbirthday: 1990-05-12\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![date_field("birthday")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(
            result.violations.is_empty(),
            "expected no violations, got: {:?}",
            result.violations
        );
    }

    #[test]
    fn date_field_rejects_invalid_calendar_date() {
        // "2024-13-45" is RFC 3339 *syntax* but jsonschema's date validator
        // catches invalid month/day.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nbirthday: \"2024-13-45\"\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![date_field("birthday")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "birthday" && matches!(v.kind, ViolationKind::WrongType))
            .expect("expected WrongType violation on birthday");
        assert!(v.rule.contains("format date"), "got rule: {}", v.rule);
    }

    #[test]
    fn date_field_rejects_non_iso_format() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nbirthday: \"05/12/1990\"\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![date_field("birthday")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "birthday" && matches!(v.kind, ViolationKind::WrongType));
        assert!(
            v.is_some(),
            "expected WrongType violation on non-ISO date, got: {:?}",
            result.violations
        );
    }

    #[test]
    fn date_field_rejects_non_string_value() {
        // An integer-looking value YAML parses as an int — jsonschema's `string`
        // type check fires before format validation.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nbirthday: 42\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![date_field("birthday")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "birthday" && matches!(v.kind, ViolationKind::WrongType));
        assert!(v.is_some(), "violations: {:?}", result.violations);
    }

    // ===== DateTime type (TODO-0007 Wave 3) =====

    fn datetime_field(name: &str) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: FieldTypeSerde::Scalar("DateTime".into()),
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    #[test]
    fn datetime_field_accepts_rfc3339_datetime() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nsynced_at: \"2024-01-15T14:30:00Z\"\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![datetime_field("synced_at")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        assert!(
            result.violations.is_empty(),
            "expected no violations, got: {:?}",
            result.violations
        );
    }

    #[test]
    fn datetime_field_rejects_naive_datetime() {
        // Missing tz offset — not valid RFC 3339.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nsynced_at: \"2024-01-15T14:30:00\"\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![datetime_field("synced_at")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "synced_at" && matches!(v.kind, ViolationKind::WrongType));
        assert!(
            v.is_some(),
            "expected WrongType on naive datetime; got: {:?}",
            result.violations
        );
    }

    #[test]
    fn datetime_field_rejects_pure_date() {
        // RFC 3339 full-date is not a valid datetime.
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("blog")).unwrap();
        fs::write(
            tmp.path().join("blog/post.md"),
            "---\nsynced_at: \"2024-01-15\"\n---\n# Body",
        )
        .unwrap();
        write_toml(tmp.path(), vec![datetime_field("synced_at")], vec![]);

        let step = run(tmp.path(), true, false, None);
        let result = unwrap_check(&step);
        let v = result
            .violations
            .iter()
            .find(|v| v.field == "synced_at" && matches!(v.kind, ViolationKind::WrongType));
        assert!(v.is_some(), "violations: {:?}", result.violations);
    }
}
