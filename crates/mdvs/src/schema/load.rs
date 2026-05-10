//! Schema loading and source resolution.
//!
//! - [`load_schema`] parses a schema file at a given path. Format is
//!   determined by file extension: `.json` uses `serde_json`, `.toml` uses
//!   the `tomljson` crate. Other extensions error.
//! - [`resolve_schema`] picks the canonical JSON Schema for a command run:
//!   if a `--schema` CLI override was provided, load it; otherwise translate
//!   the project's `mdvs.toml` DSL via [`dsl_to_canonical`]. Either source
//!   must be available; both absent is an error.
//!
//! The validation gate ([`validate_mdvs_schema`]) is **not** called here.
//! Callers run it after loading — typically `init --schema` and `check --schema`
//! gate the resolved value against the mdvs subset before using it.

use crate::schema::config::MdvsToml;
use crate::schema::json_schema::dsl_to_canonical;
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Load a schema file and parse it according to its extension.
///
/// Supported extensions (case-insensitive): `.json`, `.toml`.
/// Any other extension or no extension is an error.
pub(crate) fn load_schema(path: &Path) -> anyhow::Result<Value> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase());
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("failed to read schema '{}': {e}", path.display()))?;
    match ext.as_deref() {
        Some("json") => serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("invalid JSON in '{}': {e}", path.display())),
        Some("toml") => tomljson::from_str(&content)
            .map_err(|e| anyhow::anyhow!("invalid TOML in '{}': {e}", path.display())),
        Some(other) => anyhow::bail!(
            "unsupported schema format '.{other}' for '{}' — only .json and .toml are supported",
            path.display()
        ),
        None => anyhow::bail!(
            "schema file '{}' has no extension — append .json or .toml",
            path.display()
        ),
    }
}

/// Resolve the canonical JSON Schema for a command run.
///
/// Precedence: `cli_override` wins. Falls back to translating `mdvs.toml`
/// via [`dsl_to_canonical`]. If neither source is provided, errors.
///
/// The result is **not** validated against the mdvs subset — callers run
/// [`validate_mdvs_schema`](crate::schema::json_schema::validate_mdvs_schema)
/// after resolution.
///
/// Currently unused; step 9 (export-schema) and step 13 (overlay synthesis)
/// are the planned consumers.
#[allow(dead_code)]
pub(crate) fn resolve_schema(
    cli_override: Option<&Path>,
    toml: Option<&MdvsToml>,
) -> anyhow::Result<Value> {
    if let Some(path) = cli_override {
        return load_schema(path);
    }
    if let Some(t) = toml {
        return Ok(dsl_to_canonical(t));
    }
    anyhow::bail!("no schema source available — provide --schema or an mdvs.toml")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::config::{FieldsConfig, TomlField, UpdateConfig};
    use crate::schema::shared::{FieldTypeSerde, ScanConfig};
    use serde_json::json;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
    }

    fn empty_toml() -> MdvsToml {
        MdvsToml {
            scan: ScanConfig {
                glob: "**".into(),
                include_bare_files: false,
                skip_gitignore: false,
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

    // ------------------------------------------------------------------------
    // load_schema
    // ------------------------------------------------------------------------

    #[test]
    fn load_json_happy_path() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "schema.json",
            r#"{"type": "object", "properties": {"title": {"type": "string"}}}"#,
        );
        let v = load_schema(&path).unwrap();
        assert_eq!(v["type"], "object");
        assert_eq!(v["properties"]["title"]["type"], "string");
    }

    #[test]
    fn load_toml_happy_path() {
        let dir = TempDir::new().unwrap();
        // Use tomljson to serialize a known JSON Schema, then re-load it.
        let schema = json!({"type": "object", "properties": {"title": {"type": "string"}}});
        let toml_str = tomljson::to_string(&schema).unwrap();
        let path = write(&dir, "schema.toml", &toml_str);
        let v = load_schema(&path).unwrap();
        assert_eq!(v, schema);
    }

    #[test]
    fn load_uppercase_extension_works() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "schema.JSON", r#"{"type": "string"}"#);
        let v = load_schema(&path).unwrap();
        assert_eq!(v["type"], "string");
    }

    #[test]
    fn load_rejects_yaml() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "schema.yaml", "type: string");
        let err = load_schema(&path).unwrap_err().to_string();
        assert!(err.contains("unsupported schema format"), "got: {err}");
        assert!(err.contains(".yaml"), "got: {err}");
    }

    #[test]
    fn load_rejects_no_extension() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "schema", r#"{"type": "string"}"#);
        let err = load_schema(&path).unwrap_err().to_string();
        assert!(err.contains("no extension"), "got: {err}");
    }

    #[test]
    fn load_rejects_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let err = load_schema(&path).unwrap_err().to_string();
        assert!(err.contains("failed to read"), "got: {err}");
    }

    #[test]
    fn load_rejects_malformed_json() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "bad.json", "{not valid json");
        let err = load_schema(&path).unwrap_err().to_string();
        assert!(err.contains("invalid JSON"), "got: {err}");
    }

    #[test]
    fn load_rejects_malformed_toml() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "bad.toml", "this is not = = = valid");
        let err = load_schema(&path).unwrap_err().to_string();
        assert!(err.contains("invalid TOML"), "got: {err}");
    }

    // ------------------------------------------------------------------------
    // resolve_schema
    // ------------------------------------------------------------------------

    #[test]
    fn resolve_cli_override_wins() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "schema.json", r#"{"type": "string"}"#);
        let toml = MdvsToml {
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("Integer".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                }],
                max_categories: 10,
                min_category_repetition: 3,
            },
            ..empty_toml()
        };
        let v = resolve_schema(Some(&path), Some(&toml)).unwrap();
        // CLI override wins: the loaded file is `{"type": "string"}`, not
        // the canonical form of the toml (which would be an object schema).
        assert_eq!(v["type"], "string");
        assert!(v.get("properties").is_none());
    }

    #[test]
    fn resolve_falls_back_to_toml() {
        let toml = MdvsToml {
            fields: FieldsConfig {
                ignore: vec![],
                field: vec![TomlField {
                    name: "title".into(),
                    field_type: FieldTypeSerde::Scalar("String".into()),
                    allowed: vec!["**".into()],
                    required: vec![],
                    nullable: false,
                    constraints: None,
                    preprocess: vec![],
                }],
                max_categories: 10,
                min_category_repetition: 3,
            },
            ..empty_toml()
        };
        let v = resolve_schema(None, Some(&toml)).unwrap();
        assert_eq!(v["type"], "object");
        assert_eq!(v["properties"]["title"]["type"], "string");
    }

    #[test]
    fn resolve_neither_source_errors() {
        let err = resolve_schema(None, None).unwrap_err().to_string();
        assert!(err.contains("no schema source"), "got: {err}");
    }

    #[test]
    fn resolve_cli_override_load_error_propagates() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");
        let toml = empty_toml();
        let err = resolve_schema(Some(&path), Some(&toml))
            .unwrap_err()
            .to_string();
        // Should propagate the load error, not fall back to toml.
        assert!(err.contains("failed to read"), "got: {err}");
    }
}
