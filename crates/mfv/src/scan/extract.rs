use gray_matter::engine::{TOML, YAML};
use gray_matter::{Matter, ParsedEntity};
use mdvs_schema::FrontmatterFormat;
use serde_json::Value;

/// Extract frontmatter and body from markdown content.
///
/// The `format` parameter controls which delimiter types are recognized:
/// - `Both`: detect TOML (`+++`) vs YAML (`---`) automatically (default)
/// - `Yaml`: only recognize YAML (`---`); TOML-delimited files treated as bare
/// - `Toml`: only recognize TOML (`+++`); YAML-delimited files treated as bare
pub fn extract_frontmatter(content: &str, format: FrontmatterFormat) -> (Option<Value>, String) {
    match (content.trim_start().as_bytes(), format) {
        ([b'+', b'+', b'+', ..], FrontmatterFormat::Both | FrontmatterFormat::Toml) => {
            let mut matter = Matter::<TOML>::new();
            matter.delimiter = "+++".to_string();
            extract_data(matter.parse(content))
        }
        ([b'-', b'-', b'-', ..], FrontmatterFormat::Both | FrontmatterFormat::Yaml) => {
            extract_data(Matter::<YAML>::new().parse(content))
        }
        _ => (None, content.to_string()),
    }
}

fn extract_data(parsed: ParsedEntity) -> (Option<Value>, String) {
    let data = parsed.data.and_then(|d| {
        let json: Value = d.deserialize().ok()?;
        if json.is_object() { Some(json) } else { None }
    });
    (data, parsed.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn yaml_basic() {
        let content = "---\ntitle: Hello\ntags:\n  - rust\n---\nBody text here.";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Both);
        let fm = fm.expect("should parse YAML frontmatter");
        assert_eq!(fm["title"], json!("Hello"));
        assert_eq!(fm["tags"], json!(["rust"]));
        assert_eq!(body, "Body text here.");
    }

    #[test]
    fn toml_basic() {
        let content = "+++\ntitle = \"Hello\"\ndraft = true\n+++\nBody text here.";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Both);
        let fm = fm.expect("should parse TOML frontmatter");
        assert_eq!(fm["title"], json!("Hello"));
        assert_eq!(fm["draft"], json!(true));
        assert_eq!(body, "Body text here.");
    }

    #[test]
    fn no_frontmatter() {
        let content = "Just some plain text.\nNo delimiters here.";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Both);
        assert!(fm.is_none());
        assert_eq!(body, content);
    }

    #[test]
    fn yaml_empty_block() {
        let content = "---\n---\nbody here";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Both);
        assert!(fm.is_none());
        assert_eq!(body, "body here");
    }

    #[test]
    fn toml_unclosed_delimiter() {
        let content = "+++\ntitle = \"Hello\"\nNo closing delimiter.";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        assert!(fm.is_none());
    }

    #[test]
    fn toml_malformed() {
        let content = "+++\n[invalid toml = = =\n+++\nbody";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        assert!(fm.is_none());
    }

    #[test]
    fn yaml_non_object() {
        let content = "---\njust a string\n---\nbody";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        assert!(
            fm.is_none(),
            "scalar YAML should be filtered by is_object() check"
        );
    }

    #[test]
    fn yaml_types_preserved() {
        let content = "---\nbool_val: true\nint_val: 42\nfloat_val: 3.14\narr_val:\n  - one\n  - two\nnested:\n  key: value\n---\n";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        let fm = fm.expect("should parse");
        assert_eq!(fm["bool_val"], json!(true));
        assert_eq!(fm["int_val"], json!(42));
        assert_eq!(fm["float_val"], json!(3.14));
        assert_eq!(fm["arr_val"], json!(["one", "two"]));
        assert_eq!(fm["nested"]["key"], json!("value"));
    }

    #[test]
    fn toml_types_preserved() {
        let content = "+++\nbool_val = true\nint_val = 42\nfloat_val = 3.14\narr_val = [\"one\", \"two\"]\ndt = 2025-06-12T10:00:00\n+++\n";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        let fm = fm.expect("should parse");
        assert_eq!(fm["bool_val"], json!(true));
        assert_eq!(fm["int_val"], json!(42));
        assert_eq!(fm["float_val"], json!(3.14));
        assert_eq!(fm["arr_val"], json!(["one", "two"]));
        // TOML datetime becomes string
        assert!(fm["dt"].is_string());
    }

    #[test]
    fn toml_nan_becomes_null() {
        let content = "+++\nval = nan\n+++\n";
        let (fm, _body) = extract_frontmatter(content, FrontmatterFormat::Both);
        let fm = fm.expect("should parse");
        assert_eq!(
            fm["val"],
            json!(null),
            "NaN should become null via from_f64"
        );
    }

    #[test]
    fn yaml_skipped_when_toml_only() {
        let content = "---\ntitle: Hello\n---\nBody";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Toml);
        assert!(
            fm.is_none(),
            "YAML frontmatter should be skipped in Toml mode"
        );
        assert_eq!(body, content);
    }

    #[test]
    fn toml_skipped_when_yaml_only() {
        let content = "+++\ntitle = \"Hello\"\n+++\nBody";
        let (fm, body) = extract_frontmatter(content, FrontmatterFormat::Yaml);
        assert!(
            fm.is_none(),
            "TOML frontmatter should be skipped in Yaml mode"
        );
        assert_eq!(body, content);
    }
}
