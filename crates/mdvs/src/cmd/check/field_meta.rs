//! Per-field precomputation: compile validators, globsets, and field
//! types once per `validate()` call instead of redoing the work in every
//! `(field, file)` inner-loop iteration.
//!
//! Two artifacts, both consumed by [`super::validate::validate`]:
//! - [`FieldValidators`] — a `jsonschema::Validator` per field, built
//!   from the canonical JSON Schema's leaf subschemas.
//! - [`FieldMeta`] — compiled `GlobSet`s for `allowed` / `required` plus
//!   a cached `FieldType`, keyed by field name.

use crate::discover::field_type::FieldType;
use crate::schema::config::MdvsToml;
use crate::schema::json_schema::{dsl_to_canonical, is_intermediate_object};
use globset::{Glob, GlobSet, GlobSetBuilder};
use jsonschema::Validator;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Per-field precomputed metadata, built once per `validate()` call.
pub(super) struct FieldMeta {
    /// Compiled `GlobSet` for `allowed`; matches every path the field is
    /// permitted at. Empty patterns yield an empty `GlobSet` whose
    /// `is_match` is always false — same semantics as
    /// `matches_any_glob(&[], _)`.
    pub(super) allowed: GlobSet,
    /// Compiled `GlobSet` for `required`.
    pub(super) required: GlobSet,
    /// Converted `FieldType`, cached so the per-file loop doesn't re-parse.
    /// `None` if conversion fails (preserves prior behavior of silently
    /// skipping strict-Float precheck on malformed declarations).
    pub(super) field_type: Option<FieldType>,
}

/// Build per-field metadata for every declared field. Compiles `GlobSet`s
/// once (vs. per-call inside `matches_any_glob`) and pre-converts the
/// `FieldType`.
pub(super) fn build_field_metas(config: &MdvsToml) -> HashMap<String, FieldMeta> {
    let mut out = HashMap::with_capacity(config.fields.field.len());
    for field in &config.fields.field {
        out.insert(
            field.name.clone(),
            FieldMeta {
                allowed: build_globset(&field.allowed),
                required: build_globset(&field.required),
                field_type: FieldType::try_from(&field.field_type).ok(),
            },
        );
    }
    out
}

/// Compile a slice of glob patterns into a `GlobSet`. Patterns that fail
/// to parse are silently dropped — matching the prior `matches_any_glob`
/// behavior (`.ok()` ignored bad patterns). If `GlobSet::build` itself
/// fails (it shouldn't with successfully-parsed `Glob`s), the result is
/// an empty `GlobSet` (matches nothing) so the field is treated as if it
/// had no patterns; this is safer than panicking inside `validate`.
fn build_globset(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

/// Per-field compiled `jsonschema::Validator`s.
///
/// Each leaf field has its own validator compiled from the field's
/// subschema in the canonical JSON Schema, not the root schema. Avoids
/// re-walking `properties` on every (field, file) iteration AND lets us
/// validate values that live deep inside nested frontmatter (dotted-name
/// leaves) without re-synthesizing the full document context.
///
/// Per-field validation skips a few things the root-schema approach would
/// catch (cross-field constraints, `additionalProperties` at root) but
/// those don't apply at the leaf level anyway:
/// - The root schema is `type: object, additionalProperties: true` and
///   has no `required`, so jsonschema would never produce `Required` or
///   `AdditionalProperties` errors anyway.
/// - The field name is known a priori from the dispatch loop, no need to
///   parse `instance_path`.
pub(super) struct FieldValidators {
    per_field: HashMap<String, Validator>,
}

impl FieldValidators {
    pub(super) fn build(config: &MdvsToml) -> anyhow::Result<Self> {
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

    pub(super) fn get(&self, field_name: &str) -> Option<&Validator> {
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
        if is_intermediate_object(sub) {
            if let Some(inner) = sub.get("properties").and_then(Value::as_object) {
                walk_schema_leaves(inner, &full, out);
            }
        } else {
            out.insert(full, sub.clone());
        }
    }
}

fn strip_x_mdvs(mut value: Value) -> Value {
    if let Value::Object(map) = &mut value {
        map.remove("x-mdvs");
        // Recurse into nested schemas (properties, items, additionalProperties).
        if let Some(props) = map.get_mut("properties").and_then(Value::as_object_mut) {
            for (_, v) in props.iter_mut() {
                *v = strip_x_mdvs(v.take());
            }
        }
        if let Some(items) = map.get_mut("items") {
            *items = strip_x_mdvs(items.take());
        }
        if let Some(ap) = map.get_mut("additionalProperties")
            && ap.is_object()
        {
            *ap = strip_x_mdvs(ap.take());
        }
    }
    value
}
