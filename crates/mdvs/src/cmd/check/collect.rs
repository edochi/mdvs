//! Violation collection and `ValidationError` → `ViolationKind` mapping.
//!
//! [`collect_violations`] turns the per-(field, kind, rule) accumulator
//! into a deterministically-sorted `Vec<FieldViolation>` (per the
//! contract that `mdvs check` output is byte-stable across runs).
//!
//! [`map_validation_error`] is the big match that translates
//! `jsonschema::ValidationError` kinds into the mdvs surface. Called by
//! `super::validate::check_field_values` for every error the validator
//! emits.

use std::collections::HashMap;

use jsonschema::{ValidationError, error::ValidationErrorKind};
use serde_json::Value;

use crate::{
    discover::field_type::FieldType,
    output::{FieldViolation, ViolatingFile, ViolationKind},
    schema::{config::TomlField, shared::FieldTypeSerde},
};

/// Accumulator key for grouping violations by field, kind, and rule.
#[derive(PartialEq, Eq, Hash)]
pub(super) struct ViolationKey {
    pub(super) field: String,
    pub(super) kind: ViolationKind,
    pub(super) rule: String,
}

/// Carrier for [`map_validation_error`]'s output. Mapped before being
/// merged into the accumulator under a fresh [`ViolationKey`].
pub(super) struct MappedViolation {
    pub(super) kind: ViolationKind,
    pub(super) rule: String,
    pub(super) detail: Option<String>,
}

/// Convert the accumulator into a sorted `Vec<FieldViolation>`.
///
/// Outer sort: `(field, kind, rule)` — the same triple `ViolationKey`
/// groups on. Inner sort: files within each violation sorted by path.
/// Together these give a byte-stable output regardless of input file
/// order, important so machine consumers (CI, diff-against-baseline
/// tooling) can compare `mdvs check` output across runs.
pub(super) fn collect_violations(
    violations: HashMap<ViolationKey, Vec<ViolatingFile>>,
) -> Vec<FieldViolation> {
    let mut field_violations: Vec<FieldViolation> = violations
        .into_iter()
        .map(|(key, mut files)| {
            files.sort_by(|a, b| a.path.cmp(&b.path));
            FieldViolation {
                field: key.field,
                kind: key.kind,
                rule: key.rule,
                files,
            }
        })
        .collect();
    field_violations.sort_by(|a, b| {
        a.field
            .cmp(&b.field)
            .then_with(|| a.kind.cmp(&b.kind))
            .then_with(|| a.rule.cmp(&b.rule))
    });
    field_violations
}

/// Translate a `jsonschema::ValidationError` into the mdvs `ViolationKind`
/// shape, using the field's TOML config for rule strings.
///
/// The two non-mechanical cases:
/// - `Type` against a `null` instance → `NullNotAllowed` (not `WrongType`).
/// - `Pattern` mismatch → `WrongType` (no dedicated `PatternMismatch` variant
///   in v0; the rule string carries the pattern).
pub(super) fn map_validation_error(
    err: &ValidationError,
    value: &Value,
    field: &TomlField,
) -> MappedViolation {
    use ValidationErrorKind as E;
    use ViolationKind as K;

    // Resolve the actual offending instance — for top-level errors it's the
    // value we passed; for `items` errors it's at `instance_path` index N.
    let instance = resolve_instance_path(value, &err.instance_path().to_string());
    let got = |kind, rule| value_violation(kind, rule, instance);

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
        E::Enum { options } => got(K::InvalidCategory, format!("enum {options}")),
        E::Constant { expected_value } => {
            got(K::InvalidCategory, format!("const {expected_value}"))
        }
        E::Minimum { limit } => got(K::OutOfRange, format!("minimum {limit}")),
        E::Maximum { limit } => got(K::OutOfRange, format!("maximum {limit}")),
        E::ExclusiveMinimum { limit } => got(K::OutOfRange, format!("exclusiveMinimum {limit}")),
        E::ExclusiveMaximum { limit } => got(K::OutOfRange, format!("exclusiveMaximum {limit}")),
        E::MultipleOf { multiple_of } => got(K::OutOfRange, format!("multipleOf {multiple_of}")),
        E::MinLength { limit } => got(K::OutOfRange, format!("minLength {limit}")),
        E::MaxLength { limit } => got(K::OutOfRange, format!("maxLength {limit}")),
        E::Pattern { pattern } => got(K::WrongType, format!("pattern {pattern}")),
        E::Format { format } => got(K::WrongType, format!("format {format}")),
        E::MinItems { limit } => got(K::OutOfRange, format!("minItems {limit}")),
        E::MaxItems { limit } => got(K::OutOfRange, format!("maxItems {limit}")),
        E::UniqueItems => got(K::OutOfRange, "uniqueItems".to_string()),
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

/// Build the [`MappedViolation`] for a value-comparing error: `rule` names
/// the violated constraint and the detail is the offending value as
/// `got <json>`.
fn value_violation(kind: ViolationKind, rule: String, instance: &Value) -> MappedViolation {
    MappedViolation {
        kind,
        rule,
        detail: Some(format!("got {instance}")),
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

fn actual_type_name(value: &Value) -> String {
    FieldTypeSerde::from(&FieldType::from(value)).to_string()
}
