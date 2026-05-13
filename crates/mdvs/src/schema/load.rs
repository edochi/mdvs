//! Schema loading.
//!
//! [`load_schema`] parses a schema file at a given path. Format is
//! determined by file extension: `.json` uses `serde_json`, `.toml` uses
//! the `tomljson` crate. Other extensions error.
//!
//! The validation gate
//! ([`validate_mdvs_schema`](crate::schema::json_schema::validate_mdvs_schema))
//! is **not** called here. Callers run it after loading — typically
//! `init --from-jsonschema` and `check --jsonschema` gate the loaded value
//! against the mdvs subset before using it.

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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, content).unwrap();
        path
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
}
