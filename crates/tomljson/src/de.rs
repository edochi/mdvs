use crate::TomlJsonOptions;
use crate::error::{Error, Result};
use crate::ser::ROOT_KEY;
use serde_json::Value as Json;
use toml::Value as Toml;

/// Decode a TOML string to a JSON value.
///
/// Strings equal to `options.null_placeholder` (default `"__null__"`) are
/// substituted with JSON `null`. TOML datetimes (all four variants) decode
/// to JSON strings using their canonical RFC 3339 representation. A
/// top-level table containing only the reserved `__root__` key is unwrapped
/// to its inner value (the inverse of the encode-side wrapping).
///
/// Errors if the input fails to parse, or if a TOML float carries `NaN`,
/// `+inf`, or `-inf` — these cannot be represented in JSON.
pub fn from_str_with_options(s: &str, options: &TomlJsonOptions) -> Result<Json> {
    let parsed: Toml = toml::from_str(s)?;
    let placeholder = options.null_placeholder.as_str();
    let mut path_stack: Vec<String> = Vec::new();
    let value = walk(&parsed, placeholder, &mut path_stack)?;

    // Unwrap top-level `__root__` if present.
    if let Json::Object(ref obj) = value
        && obj.len() == 1
        && let Some(inner) = obj.get(ROOT_KEY)
    {
        return Ok(inner.clone());
    }

    Ok(value)
}

fn walk(v: &Toml, placeholder: &str, path_stack: &mut Vec<String>) -> Result<Json> {
    match v {
        Toml::String(s) if s == placeholder => Ok(Json::Null),
        Toml::String(s) => Ok(Json::String(s.clone())),

        Toml::Integer(i) => Ok(Json::Number((*i).into())),

        Toml::Float(f) => {
            if f.is_nan() {
                return Err(Error::FloatNotRepresentable {
                    path: format_path(path_stack),
                    kind: "NaN",
                });
            }
            if f.is_infinite() {
                let kind = if *f > 0.0 { "+inf" } else { "-inf" };
                return Err(Error::FloatNotRepresentable {
                    path: format_path(path_stack),
                    kind,
                });
            }
            Ok(serde_json::Number::from_f64(*f)
                .map(Json::Number)
                .expect("finite float must convert"))
        }

        Toml::Boolean(b) => Ok(Json::Bool(*b)),

        // All four TOML datetime variants → canonical RFC 3339 string.
        // JSON Schema represents dates/times as strings with `format: date|time|date-time`.
        Toml::Datetime(dt) => Ok(Json::String(dt.to_string())),

        Toml::Array(arr) => {
            let mut out = Vec::with_capacity(arr.len());
            for (i, item) in arr.iter().enumerate() {
                path_stack.push(i.to_string());
                let r = walk(item, placeholder, path_stack);
                path_stack.pop();
                out.push(r?);
            }
            Ok(Json::Array(out))
        }

        Toml::Table(t) => {
            let mut obj = serde_json::Map::with_capacity(t.len());
            for (k, val) in t {
                path_stack.push(escape_pointer_segment(k));
                let r = walk(val, placeholder, path_stack);
                path_stack.pop();
                obj.insert(k.clone(), r?);
            }
            Ok(Json::Object(obj))
        }
    }
}

fn format_path(segments: &[String]) -> String {
    if segments.is_empty() {
        "".to_string()
    } else {
        let mut out = String::new();
        for s in segments {
            out.push('/');
            out.push_str(s);
        }
        out
    }
}

/// Escape `/` and `~` per RFC 6901 JSON Pointer rules.
fn escape_pointer_segment(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
