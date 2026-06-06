//! Translation between the `mdvs.toml` DSL and canonical JSON Schema 2020-12,
//! plus a validation gate that rejects schemas using keywords mdvs doesn't support.
//!
//! Two entry points:
//! - [`dsl_to_canonical`] — `MdvsToml` → `serde_json::Value` (a JSON Schema document).
//! - [`validate_mdvs_schema`] — checks an arbitrary JSON Schema is within the mdvs subset.
//!
//! Path-scoped validation (per-file `allowed`/`required` globs) is carried as
//! `x-mdvs.allowed` / `x-mdvs.required` on each property; the per-file overlay
//! synthesizer (TODO-0149 step 13) turns those into actual JSON Schema `required`
//! arrays at validation time.

mod from_canonical;
mod to_canonical;
mod validate;

pub(crate) use from_canonical::canonical_to_dsl;
pub(crate) use to_canonical::dsl_to_canonical;
pub(crate) use validate::validate_mdvs_schema;

use serde_json::Value;

// Test-only imports for the round-trip and integration tests below. The
// production code in this mod.rs (just `is_intermediate_object`) doesn't
// need them, but the test mod's `use super::*;` pulls them in.
#[cfg(test)]
#[allow(unused_imports)]
use crate::discover::field_type::FieldType;
#[cfg(test)]
#[allow(unused_imports)]
use crate::schema::config::MdvsToml;
#[cfg(test)]
#[allow(unused_imports)]
use crate::schema::constraints::Constraints;
#[cfg(test)]
#[allow(unused_imports)]
use serde_json::{Map, json};
#[cfg(test)]
#[allow(unused_imports)]
use to_canonical::JSON_SCHEMA_DRAFT;

/// True if the value is the exact shape produced by
/// `to_canonical::intermediate_object_schema` (or its post-population
/// state): an object schema with a `properties` map and no `x-mdvs`
/// metadata. Used by `to_canonical`'s `insert_at_path` and by
/// `from_canonical`'s recursive walk to distinguish structural Objects
/// from leaf Object schemas (Array-of-Object inner types and similar).
pub(crate) fn is_intermediate_object(v: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    obj.get("type") == Some(&Value::String("object".into()))
        && obj.get("properties").map(Value::is_object).unwrap_or(false)
        && !obj.contains_key("x-mdvs")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::{FieldTypeSerde, FrontmatterFormat, ScanConfig};

    fn empty_toml() -> MdvsToml {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
                frontmatter_format: FrontmatterFormat::Auto,
            },
            update: UpdateConfig::default(),
            check: None,
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![],
                max_categories: 10,
                min_category_repetition: 3,
            },
            embedding_model: None,
            chunking: None,
            build: None,
            search: None,
        }
    }

    fn with_fields(fields: Vec<TomlField>) -> MdvsToml {
        let mut t = empty_toml();
        t.fields.field = fields;
        t
    }

    fn field(name: &str, ft: FieldTypeSerde) -> TomlField {
        TomlField {
            name: name.into(),
            field_type: ft,
            allowed: vec!["**".into()],
            required: vec![],
            nullable: false,
            constraints: None,
            preprocess: vec![],
        }
    }

    // ------------------------------------------------------------------------
    // Group A — dsl_to_canonical happy path
    // ------------------------------------------------------------------------

    #[test]
    fn empty_config_produces_empty_object_schema() {
        let out = dsl_to_canonical(&empty_toml());
        assert_eq!(out["$schema"], JSON_SCHEMA_DRAFT);
        assert_eq!(out["type"], "object");
        assert_eq!(out["additionalProperties"], true);
        assert_eq!(out["properties"], json!({}));
    }

    #[test]
    fn string_field_simple_strict() {
        // String fields emit strict `{"type": "string"}` — coercion is the
        // preprocessor's job, not the schema's.
        let toml = with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["title"], json!({"type": "string"}));
    }

    #[test]
    fn integer_field_simple() {
        let toml = with_fields(vec![field(
            "count",
            FieldTypeSerde::Scalar("Integer".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["count"], json!({"type": "integer"}));
    }

    #[test]
    fn float_field_simple() {
        let toml = with_fields(vec![field("score", FieldTypeSerde::Scalar("Float".into()))]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["score"], json!({"type": "number"}));
    }

    #[test]
    fn boolean_field_simple() {
        let toml = with_fields(vec![field(
            "draft",
            FieldTypeSerde::Scalar("Boolean".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["draft"], json!({"type": "boolean"}));
    }

    #[test]
    fn date_field_emits_string_with_format() {
        let toml = with_fields(vec![field(
            "birthday",
            FieldTypeSerde::Scalar("Date".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["birthday"],
            json!({"type": "string", "format": "date"})
        );
    }

    #[test]
    fn array_of_date_emits_items_format() {
        let toml = with_fields(vec![field(
            "milestones",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Date".into())),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["milestones"],
            json!({
                "type": "array",
                "items": {"type": "string", "format": "date"}
            })
        );
    }

    #[test]
    fn nullable_date_emits_union_with_format() {
        let mut f = field("birthday", FieldTypeSerde::Scalar("Date".into()));
        f.nullable = true;
        let toml = with_fields(vec![f]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["birthday"],
            json!({"type": ["string", "null"], "format": "date"})
        );
    }

    #[test]
    fn canonical_to_dsl_recognises_format_date() {
        // Build a canonical schema by hand, run canonical_to_dsl, expect Date.
        let schema = json!({
            "$schema": JSON_SCHEMA_DRAFT,
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "birthday": {"type": "string", "format": "date"}
            },
        });
        let imported = canonical_to_dsl(&schema).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "birthday");
        assert_eq!(fields[0].field_type, FieldTypeSerde::Scalar("Date".into()));
    }

    #[test]
    fn dsl_to_canonical_round_trip_with_date() {
        let toml = with_fields(vec![field(
            "birthday",
            FieldTypeSerde::Scalar("Date".into()),
        )]);
        let canonical = dsl_to_canonical(&toml);
        let imported = canonical_to_dsl(&canonical).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "birthday");
        assert_eq!(fields[0].field_type, FieldTypeSerde::Scalar("Date".into()));
    }

    // --- DateTime translator tests (TODO-0007 Wave 3) ---

    #[test]
    fn datetime_field_emits_string_with_format_date_time() {
        let toml = with_fields(vec![field(
            "synced_at",
            FieldTypeSerde::Scalar("DateTime".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["synced_at"],
            json!({"type": "string", "format": "date-time"})
        );
    }

    #[test]
    fn array_of_datetime_emits_items_format() {
        let toml = with_fields(vec![field(
            "events",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("DateTime".into())),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["events"],
            json!({
                "type": "array",
                "items": {"type": "string", "format": "date-time"}
            })
        );
    }

    #[test]
    fn nullable_datetime_emits_union_with_format() {
        let mut f = field("synced_at", FieldTypeSerde::Scalar("DateTime".into()));
        f.nullable = true;
        let toml = with_fields(vec![f]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["synced_at"],
            json!({"type": ["string", "null"], "format": "date-time"})
        );
    }

    #[test]
    fn canonical_to_dsl_recognises_format_date_time() {
        let schema = json!({
            "$schema": JSON_SCHEMA_DRAFT,
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "synced_at": {"type": "string", "format": "date-time"}
            },
        });
        let imported = canonical_to_dsl(&schema).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "synced_at");
        assert_eq!(
            fields[0].field_type,
            FieldTypeSerde::Scalar("DateTime".into())
        );
    }

    #[test]
    fn dsl_to_canonical_round_trip_with_datetime() {
        let toml = with_fields(vec![field(
            "synced_at",
            FieldTypeSerde::Scalar("DateTime".into()),
        )]);
        let canonical = dsl_to_canonical(&toml);
        let imported = canonical_to_dsl(&canonical).unwrap();
        let fields = &imported.fields;
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name, "synced_at");
        assert_eq!(
            fields[0].field_type,
            FieldTypeSerde::Scalar("DateTime".into())
        );
    }

    #[test]
    fn nullable_string_field_emits_union_type() {
        // String + nullable=true: standard `["string", "null"]` union.
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({"type": ["string", "null"]})
        );
    }

    #[test]
    fn nullable_integer_field_emits_union_type() {
        // Non-String types follow the standard union-with-null pattern.
        let mut f = field("count", FieldTypeSerde::Scalar("Integer".into()));
        f.nullable = true;
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["count"],
            json!({"type": ["integer", "null"]})
        );
    }

    #[test]
    fn array_of_strings_field_strict() {
        let toml = with_fields(vec![field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["tags"],
            json!({"type": "array", "items": {"type": "string"}})
        );
    }

    #[test]
    fn array_of_object_emits_items_properties() {
        // After TODO-0097 step 3, Object as a top-level type is rejected by
        // MdvsToml::validate invariant 6 (top-level structure is expressed
        // via dotted-name leaves). Object survives only as an Array inner
        // type — and now the translator emits proper `items.properties`
        // children rather than the pre-Wave-C `additionalProperties: true`
        // placeholder.
        use std::collections::BTreeMap;
        let toml = with_fields(vec![field(
            "readings",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Object {
                    object: BTreeMap::from([
                        ("author".into(), FieldTypeSerde::Scalar("String".into())),
                        ("version".into(), FieldTypeSerde::Scalar("Integer".into())),
                    ]),
                }),
            },
        )]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(
            out["properties"]["readings"],
            json!({
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "author": {"type": "string"},
                        "version": {"type": "integer"},
                    },
                }
            })
        );
    }

    #[test]
    fn categories_constraint_emits_enum() {
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["status"],
            json!({"type": "string", "enum": ["draft", "published"]})
        );
    }

    #[test]
    fn nullable_categorical_appends_null_to_enum() {
        // nullable=true + categories appends null to enum so null passes the
        // categorical check (matches the existing semantic).
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["status"],
            json!({"type": ["string", "null"], "enum": ["draft", "published", null]})
        );
    }

    #[test]
    fn range_constraint_emits_minimum_maximum() {
        let mut f = field("rating", FieldTypeSerde::Scalar("Integer".into()));
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(5)),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["rating"],
            json!({"type": "integer", "minimum": 0, "maximum": 5})
        );
    }

    #[test]
    fn length_constraint_emits_min_max_length() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(3),
            max_length: Some(64),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({"type": "string", "minLength": 3, "maxLength": 64})
        );
    }

    #[test]
    fn pattern_constraint_emits_pattern_keyword() {
        let mut f = field("slug", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            pattern: Some("^[a-z0-9-]+$".into()),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["slug"],
            json!({"type": "string", "pattern": "^[a-z0-9-]+$"})
        );
    }

    #[test]
    fn length_and_pattern_combined() {
        let mut f = field("token", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(8),
            max_length: Some(8),
            pattern: Some("^[A-Z]{8}$".into()),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["token"],
            json!({
                "type": "string",
                "minLength": 8,
                "maxLength": 8,
                "pattern": "^[A-Z]{8}$"
            })
        );
    }

    #[test]
    fn array_string_length_applies_to_items() {
        let mut f = field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        );
        f.constraints = Some(Constraints {
            min_length: Some(2),
            max_length: Some(20),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["tags"],
            json!({
                "type": "array",
                "items": {"type": "string", "minLength": 2, "maxLength": 20}
            })
        );
    }

    #[test]
    fn array_constraint_applies_to_items() {
        let mut f = field(
            "scores",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Integer".into())),
            },
        );
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(100)),
            ..Default::default()
        });
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["scores"],
            json!({
                "type": "array",
                "items": {"type": "integer", "minimum": 0, "maximum": 100}
            })
        );
    }

    #[test]
    fn path_scoping_emitted_as_x_mdvs() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.allowed = vec!["blog/**".into(), "notes/**".into()];
        f.required = vec!["blog/**".into()];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["title"],
            json!({
                "type": "string",
                "x-mdvs": {
                    "allowed": ["blog/**", "notes/**"],
                    "required": ["blog/**"]
                }
            })
        );
    }

    #[test]
    fn default_allowed_omits_x_mdvs_block() {
        // allowed = ["**"] (default) and no required → no x-mdvs at all.
        let out = dsl_to_canonical(&with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]));
        assert!(out["properties"]["title"].get("x-mdvs").is_none());
    }

    #[test]
    fn ignore_list_produces_empty_subschemas() {
        let mut t = empty_toml();
        t.fields.ignore = vec!["internal_id".into(), "draft_meta".into()];
        let out = dsl_to_canonical(&t);
        assert_eq!(out["properties"]["internal_id"], json!({}));
        assert_eq!(out["properties"]["draft_meta"], json!({}));
    }

    #[test]
    fn no_root_required_array_emitted() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.required = vec!["**".into()];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert!(out.get("required").is_none());
    }

    // ------------------------------------------------------------------------
    // Group B — dsl_to_canonical output passes the gate
    // ------------------------------------------------------------------------

    #[test]
    fn gate_accepts_empty_schema() {
        let out = dsl_to_canonical(&empty_toml());
        assert!(validate_mdvs_schema(&out).is_ok());
    }

    #[test]
    fn gate_accepts_full_schema() {
        use std::collections::BTreeMap;
        let mut f1 = field("title", FieldTypeSerde::Scalar("String".into()));
        f1.required = vec!["**".into()];
        let mut f2 = field("status", FieldTypeSerde::Scalar("String".into()));
        f2.constraints = Some(Constraints {
            categories: Some(vec![toml::Value::String("draft".into())]),
            ..Default::default()
        });
        let f3 = field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        );
        let f4 = field(
            "meta",
            FieldTypeSerde::Object {
                object: BTreeMap::from([(
                    "author".into(),
                    FieldTypeSerde::Scalar("String".into()),
                )]),
            },
        );
        let out = dsl_to_canonical(&with_fields(vec![f1, f2, f3, f4]));
        assert!(validate_mdvs_schema(&out).is_ok(), "produced: {out}");
    }

    // ------------------------------------------------------------------------
    // Group C — gate rejects denied keywords and structures
    // ------------------------------------------------------------------------

    fn assert_rejects(schema: Value, expected_substr: &str) {
        let err =
            validate_mdvs_schema(&schema).expect_err(&format!("expected rejection for: {schema}"));
        assert!(
            err.contains(expected_substr),
            "expected error containing {expected_substr:?}, got: {err}"
        );
    }

    #[test]
    fn gate_rejects_one_of() {
        assert_rejects(
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}),
            "'oneOf' is not supported",
        );
    }

    #[test]
    fn gate_rejects_any_of() {
        assert_rejects(json!({"anyOf": []}), "'anyOf' is not supported");
    }

    #[test]
    fn gate_rejects_all_of() {
        assert_rejects(json!({"allOf": []}), "'allOf' is not supported");
    }

    #[test]
    fn gate_rejects_not() {
        assert_rejects(json!({"not": {}}), "'not' is not supported");
    }

    #[test]
    fn gate_rejects_if_then_else() {
        assert_rejects(json!({"if": {}, "then": {}}), "'if' is not supported");
    }

    #[test]
    fn gate_rejects_ref() {
        assert_rejects(json!({"$ref": "#/foo"}), "'$ref' is not supported");
    }

    #[test]
    fn gate_rejects_defs() {
        assert_rejects(json!({"$defs": {}}), "'$defs' is not supported");
    }

    #[test]
    fn gate_rejects_pattern_properties() {
        assert_rejects(
            json!({"patternProperties": {"^x": {}}}),
            "'patternProperties' is not supported",
        );
    }

    #[test]
    fn gate_rejects_prefix_items() {
        assert_rejects(json!({"prefixItems": []}), "'prefixItems' is not supported");
    }

    #[test]
    fn gate_rejects_unsupported_format() {
        assert_rejects(
            json!({"format": "email"}),
            "format 'email' is not supported",
        );
    }

    #[test]
    fn gate_accepts_date_time_format() {
        assert!(validate_mdvs_schema(&json!({"format": "date-time"})).is_ok());
    }

    #[test]
    fn gate_rejects_non_string_format() {
        assert_rejects(json!({"format": 42}), "'format' must be a string");
    }

    #[test]
    fn gate_accepts_format_date() {
        assert!(validate_mdvs_schema(&json!({"format": "date"})).is_ok());
    }

    #[test]
    fn gate_rejects_unknown_root_keyword() {
        assert_rejects(
            json!({"madeUpKeyword": true}),
            "unknown keyword 'madeUpKeyword'",
        );
    }

    #[test]
    fn gate_accepts_x_mdvs_at_any_property_depth() {
        // After TODO-0097 step 3, dotted-name leaves can live at any depth.
        // The gate accepts property-level x-mdvs everywhere; only the root
        // location is gated to schema-level subkeys.
        assert!(
            validate_mdvs_schema(&json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "cal": {
                        "type": "object",
                        "additionalProperties": true,
                        "properties": {
                            "wavelength": {
                                "type": "number",
                                "x-mdvs": {"allowed": ["**"]}
                            }
                        }
                    }
                }
            }))
            .is_ok()
        );

        // Same allowance applies inside `items` (Array element schemas
        // are now valid property locations).
        assert!(
            validate_mdvs_schema(&json!({
                "type": "object",
                "additionalProperties": true,
                "properties": {
                    "tags": {
                        "type": "array",
                        "items": {
                            "type": "string",
                            "x-mdvs": {"allowed": ["**"]}
                        }
                    }
                }
            }))
            .is_ok()
        );
    }

    #[test]
    fn gate_rejects_unknown_x_mdvs_subkey() {
        assert_rejects(
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "x-mdvs": {"thirdEye": true}
                    }
                }
            }),
            "unknown 'x-mdvs.thirdEye'",
        );
    }

    #[test]
    fn gate_rejects_property_subkey_at_root() {
        // 'allowed' is a property-level x-mdvs sub-key, not schema-level.
        assert_rejects(
            json!({
                "type": "object",
                "x-mdvs": {"allowed": ["**"]}
            }),
            "unknown 'x-mdvs.allowed'",
        );
    }

    #[test]
    fn gate_accepts_schema_level_x_mdvs() {
        // 'preprocess' and 'definitions' are valid at root.
        let schema = json!({
            "type": "object",
            "x-mdvs": {"preprocess": [], "definitions": {}}
        });
        assert!(validate_mdvs_schema(&schema).is_ok());
    }

    #[test]
    fn gate_accepts_metadata_keywords() {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://example.com/foo",
            "title": "Foo",
            "description": "A foo schema",
            "type": "object"
        });
        assert!(validate_mdvs_schema(&schema).is_ok());
    }

    // ------------------------------------------------------------------------
    // canonical_to_dsl — reverse translator (TODO-0149 step 10)
    // ------------------------------------------------------------------------

    fn roundtrip(toml: MdvsToml) {
        let canonical = dsl_to_canonical(&toml);
        let import = canonical_to_dsl(&canonical).expect("reverse translation");
        // serde_json::Map preserves insertion order on round-trip but
        // dsl_to_canonical merges fields + ignore into a single `properties`
        // map; reading it back loses the original ordering distinction.
        // Compare as sorted sets.
        let mut got_fields = import.fields;
        got_fields.sort_by(|a, b| a.name.cmp(&b.name));
        let mut exp_fields = toml.fields.field.clone();
        exp_fields.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(got_fields, exp_fields, "fields mismatch");
        let mut got_ignore = import.ignore;
        got_ignore.sort();
        let mut exp_ignore = toml.fields.ignore.clone();
        exp_ignore.sort();
        assert_eq!(got_ignore, exp_ignore, "ignore mismatch");
    }

    #[test]
    fn roundtrip_string_field() {
        roundtrip(with_fields(vec![field(
            "title",
            FieldTypeSerde::Scalar("String".into()),
        )]));
    }

    #[test]
    fn roundtrip_integer_field() {
        roundtrip(with_fields(vec![field(
            "count",
            FieldTypeSerde::Scalar("Integer".into()),
        )]));
    }

    #[test]
    fn roundtrip_float_field() {
        roundtrip(with_fields(vec![field(
            "score",
            FieldTypeSerde::Scalar("Float".into()),
        )]));
    }

    #[test]
    fn roundtrip_boolean_field() {
        roundtrip(with_fields(vec![field(
            "draft",
            FieldTypeSerde::Scalar("Boolean".into()),
        )]));
    }

    #[test]
    fn roundtrip_nullable_string() {
        let mut f = field("note", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_nullable_integer() {
        let mut f = field("count", FieldTypeSerde::Scalar("Integer".into()));
        f.nullable = true;
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_array_of_strings() {
        roundtrip(with_fields(vec![field(
            "tags",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
        )]));
    }

    #[test]
    fn roundtrip_categories_constraint() {
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_nullable_plus_enum_strips_appended_null() {
        // dsl_to_canonical appends null to the enum when nullable=true;
        // canonical_to_dsl must strip it back.
        let mut f = field("status", FieldTypeSerde::Scalar("String".into()));
        f.nullable = true;
        f.constraints = Some(Constraints {
            categories: Some(vec![
                toml::Value::String("draft".into()),
                toml::Value::String("published".into()),
            ]),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_range_constraint() {
        let mut f = field("rating", FieldTypeSerde::Scalar("Integer".into()));
        f.constraints = Some(Constraints {
            min: Some(toml::Value::Integer(0)),
            max: Some(toml::Value::Integer(5)),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_length_and_pattern() {
        let mut f = field("slug", FieldTypeSerde::Scalar("String".into()));
        f.constraints = Some(Constraints {
            min_length: Some(3),
            max_length: Some(64),
            pattern: Some("^[a-z0-9-]+$".into()),
            ..Default::default()
        });
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_preserves_preprocess() {
        use crate::preprocess::ValueStage;
        let mut f = field("funding", FieldTypeSerde::Scalar("String".into()));
        f.preprocess = vec![ValueStage::CoerceToString];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn dsl_to_canonical_emits_preprocess_in_x_mdvs() {
        use crate::preprocess::ValueStage;
        let mut f = field("score", FieldTypeSerde::Scalar("Float".into()));
        f.preprocess = vec![ValueStage::WidenIntToFloat];
        let out = dsl_to_canonical(&with_fields(vec![f]));
        assert_eq!(
            out["properties"]["score"]["x-mdvs"]["preprocess"],
            json!(["widen-int-to-float"])
        );
    }

    #[test]
    fn roundtrip_path_scoping() {
        let mut f = field("title", FieldTypeSerde::Scalar("String".into()));
        f.allowed = vec!["blog/**".into(), "notes/**".into()];
        f.required = vec!["blog/**".into()];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_ignore_list() {
        let mut t = empty_toml();
        t.fields.ignore = vec!["internal_id".into(), "draft_meta".into()];
        roundtrip(t);
    }

    #[test]
    fn canonical_to_dsl_rejects_missing_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "title": {"enum": ["a", "b"]}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("missing 'type'"), "got: {err}");
    }

    #[test]
    fn canonical_to_dsl_rejects_invalid_type() {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": {"type": "integer"},
                "y": {"type": "foobar"}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("unsupported type"), "got: {err}");
    }

    #[test]
    fn canonical_to_dsl_rejects_null_in_non_nullable_enum() {
        let schema = json!({
            "type": "object",
            "properties": {
                "status": {"type": "string", "enum": ["draft", null]}
            },
            "additionalProperties": true
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(err.contains("not nullable"), "got: {err}");
    }

    // ========================================================================
    // TODO-0097 step 3: dotted-name flattening
    // ========================================================================

    #[test]
    fn dsl_to_canonical_single_dotted_creates_intermediate() {
        let toml = with_fields(vec![field(
            "calibration.baseline.wavelength",
            FieldTypeSerde::Scalar("Float".into()),
        )]);
        let out = dsl_to_canonical(&toml);
        // Outer intermediate
        assert_eq!(out["properties"]["calibration"]["type"], "object");
        assert_eq!(
            out["properties"]["calibration"]["additionalProperties"],
            true
        );
        assert!(out["properties"]["calibration"].get("x-mdvs").is_none());
        // Inner intermediate
        assert_eq!(
            out["properties"]["calibration"]["properties"]["baseline"]["type"],
            "object"
        );
        // Leaf
        assert_eq!(
            out["properties"]["calibration"]["properties"]["baseline"]["properties"]["wavelength"]
                ["type"],
            "number"
        );
    }

    #[test]
    fn dsl_to_canonical_siblings_share_intermediate() {
        let toml = with_fields(vec![
            field("cal.x", FieldTypeSerde::Scalar("Float".into())),
            field("cal.y", FieldTypeSerde::Scalar("Float".into())),
        ]);
        let out = dsl_to_canonical(&toml);
        let cal_props = out["properties"]["cal"]["properties"].as_object().unwrap();
        assert!(cal_props.contains_key("x"));
        assert!(cal_props.contains_key("y"));
        assert_eq!(cal_props.len(), 2);
    }

    #[test]
    fn dsl_to_canonical_mixed_flat_and_dotted() {
        let toml = with_fields(vec![
            field("title", FieldTypeSerde::Scalar("String".into())),
            field("cal.wave", FieldTypeSerde::Scalar("Float".into())),
        ]);
        let out = dsl_to_canonical(&toml);
        assert_eq!(out["properties"]["title"]["type"], "string");
        assert_eq!(
            out["properties"]["cal"]["properties"]["wave"]["type"],
            "number"
        );
    }

    #[test]
    fn dsl_to_canonical_leaf_x_mdvs_lives_at_leaf_not_intermediate() {
        let mut f = field("cal.baseline.wave", FieldTypeSerde::Scalar("Float".into()));
        f.allowed = vec!["projects/alpha/**".into()];
        f.required = vec!["projects/alpha/**".into()];
        let toml = with_fields(vec![f]);
        let out = dsl_to_canonical(&toml);

        // Intermediates have no x-mdvs
        assert!(out["properties"]["cal"].get("x-mdvs").is_none());
        assert!(
            out["properties"]["cal"]["properties"]["baseline"]
                .get("x-mdvs")
                .is_none()
        );

        // Leaf carries x-mdvs
        let leaf_x =
            &out["properties"]["cal"]["properties"]["baseline"]["properties"]["wave"]["x-mdvs"];
        assert_eq!(leaf_x["allowed"], json!(["projects/alpha/**"]));
        assert_eq!(leaf_x["required"], json!(["projects/alpha/**"]));
    }

    #[test]
    fn canonical_to_dsl_walks_nested_into_dotted_names() {
        let schema = json!({
            "$schema": JSON_SCHEMA_DRAFT,
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "title": {"type": "string"},
                "cal": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "baseline": {
                            "type": "object",
                            "additionalProperties": true,
                            "properties": {
                                "wavelength": {"type": "number"},
                                "intensity": {"type": "number"}
                            }
                        }
                    }
                }
            }
        });
        let imported = canonical_to_dsl(&schema).unwrap();
        let names: Vec<&str> = imported.fields.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"title"));
        assert!(names.contains(&"cal.baseline.wavelength"));
        assert!(names.contains(&"cal.baseline.intensity"));
        // Intermediates do NOT become TomlFields.
        assert!(!names.contains(&"cal"));
        assert!(!names.contains(&"cal.baseline"));
    }

    #[test]
    fn roundtrip_dotted_leaf() {
        let f = field(
            "calibration.baseline.wavelength",
            FieldTypeSerde::Scalar("Float".into()),
        );
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_dotted_with_x_mdvs() {
        use crate::preprocess::ValueStage;
        let mut f = field("cal.wave", FieldTypeSerde::Scalar("Float".into()));
        f.allowed = vec!["projects/alpha/**".into()];
        f.required = vec!["projects/alpha/**".into()];
        f.preprocess = vec![ValueStage::WidenIntToFloat];
        roundtrip(with_fields(vec![f]));
    }

    #[test]
    fn roundtrip_mixed_flat_and_dotted() {
        let toml = with_fields(vec![
            field("title", FieldTypeSerde::Scalar("String".into())),
            field("cal.x", FieldTypeSerde::Scalar("Float".into())),
            field("cal.y", FieldTypeSerde::Scalar("Float".into())),
            field("meta.author", FieldTypeSerde::Scalar("String".into())),
        ]);
        roundtrip(toml);
    }

    #[test]
    fn roundtrip_array_of_object_inner_shape_preserved() {
        use std::collections::BTreeMap;
        let toml = with_fields(vec![field(
            "readings",
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Object {
                    object: BTreeMap::from([
                        ("time".into(), FieldTypeSerde::Scalar("String".into())),
                        ("value".into(), FieldTypeSerde::Scalar("Float".into())),
                    ]),
                }),
            },
        )]);
        roundtrip(toml);
    }

    #[test]
    fn canonical_to_dsl_rejects_empty_schema_at_nested_depth() {
        // An empty schema is only valid at the top level (signals an
        // ignored name). Nested empty schemas indicate a malformed input.
        let schema = json!({
            "type": "object",
            "additionalProperties": true,
            "properties": {
                "cal": {
                    "type": "object",
                    "additionalProperties": true,
                    "properties": {
                        "broken": {}
                    }
                }
            }
        });
        let err = canonical_to_dsl(&schema).unwrap_err();
        assert!(
            err.contains("empty schema is not allowed at nested depth"),
            "got: {err}"
        );
    }
}
