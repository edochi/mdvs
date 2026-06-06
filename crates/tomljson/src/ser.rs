use crate::TomlJsonOptions;
use crate::error::{Error, Result};
use serde_json::Value as Json;
use toml_writer::TomlWrite;

/// Encode a JSON value to a TOML string.
///
/// JSON `null` is encoded as `options.null_placeholder` (default `"__null__"`).
/// Top-level non-table values are wrapped under `options.root_placeholder`
/// (default `"__root__"`).
///
/// Errors if the input contains a string equal to the null placeholder, an
/// Object whose top-level keys include the root placeholder, or an integer
/// larger than `i64::MAX` (TOML integers are signed 64-bit per the TOML spec).
pub fn to_string_with_options(value: &Json, options: &TomlJsonOptions) -> Result<String> {
    let null_placeholder = options.null_placeholder.as_str();
    let root_placeholder = options.root_placeholder.as_str();
    assert_encodable(value, null_placeholder, root_placeholder, &mut Vec::new())?;

    let mut out = String::new();

    match value {
        Json::Object(obj) => {
            write_table(&mut out, &[], obj, null_placeholder)?;
        }
        other => {
            // Non-table root: wrap under the root placeholder key.
            // Use toml_writer's key() so non-bare-key strings get quoted.
            out.key(root_placeholder)?;
            out.space()?;
            out.keyval_sep()?;
            out.space()?;
            write_inline(&mut out, other, null_placeholder)?;
            out.newline()?;
        }
    }

    Ok(out)
}

// ============================================================================
// Pre-flight check: walk the value once and reject anything we can't encode.
//
// `path_stack` tracks JSON Pointer segments so error variants can report the
// offending node. Segments are pushed before recursing into a field/element
// and popped on return.
// ============================================================================

fn assert_encodable(
    v: &Json,
    null_placeholder: &str,
    root_placeholder: &str,
    path_stack: &mut Vec<String>,
) -> Result<()> {
    // Top-level collision check: if the root is an Object whose keys include
    // `root_placeholder`, encode would produce a TOML document indistinguishable
    // from a wrapped non-table root. Error before we emit anything.
    //
    // `path_stack.is_empty()` distinguishes the top-level call from recursive
    // calls on nested objects (where `__root__` as a key is fine).
    if path_stack.is_empty()
        && let Json::Object(obj) = v
        && obj.contains_key(root_placeholder)
    {
        return Err(Error::RootKeyCollision {
            placeholder: root_placeholder.to_string(),
        });
    }

    match v {
        // Json::Null is encoded as the placeholder string by `write_inline`.
        Json::Null | Json::Bool(_) => Ok(()),
        Json::Number(n) => {
            // u64-only numbers (i.e. above `i64::MAX`) cannot be represented
            // as a TOML signed-64-bit integer. `as_u64().filter(...)` binds
            // the value in one go — no separate `is_u64()` check to drift
            // from the conversion.
            if let Some(value) = n.as_u64().filter(|_| n.as_i64().is_none()) {
                Err(Error::IntegerOutOfRange {
                    path: format_path(path_stack),
                    value,
                })
            } else {
                Ok(())
            }
        }
        Json::String(s) => {
            if s == null_placeholder {
                Err(Error::PlaceholderCollision {
                    path: format_path(path_stack),
                    placeholder: null_placeholder.to_string(),
                })
            } else {
                Ok(())
            }
        }
        Json::Array(arr) => {
            for (i, item) in arr.iter().enumerate() {
                path_stack.push(i.to_string());
                let r = assert_encodable(item, null_placeholder, root_placeholder, path_stack);
                path_stack.pop();
                r?;
            }
            Ok(())
        }
        Json::Object(obj) => {
            for (k, val) in obj {
                path_stack.push(escape_pointer_segment(k));
                let r = assert_encodable(val, null_placeholder, root_placeholder, path_stack);
                path_stack.pop();
                r?;
            }
            Ok(())
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

// ============================================================================
// Inline value emission (used for arrays and inline tables, and for the root
// __root__ wrapper).
//
// At this point assert_encodable has rejected null and placeholder collisions,
// so Json::Null becomes the placeholder string verbatim.
// ============================================================================

fn write_inline<W: TomlWrite>(w: &mut W, v: &Json, placeholder: &str) -> Result<()> {
    match v {
        Json::Null => w.value(placeholder)?,
        Json::Bool(b) => w.value(*b)?,
        Json::Number(n) => {
            if let Some(i) = n.as_i64() {
                w.value(i)?;
            } else if let Some(f) = n.as_f64() {
                w.value(f)?;
            } else if let Some(value) = n.as_u64() {
                // u64 above `i64::MAX` with no `f64` representation.
                // `assert_encodable` should have rejected this upstream;
                // returning `Err` locally means a future refactor that
                // bypasses the precheck still surfaces a clean error
                // instead of panicking. `path` is empty because
                // `write_inline` doesn't thread a path stack.
                return Err(Error::IntegerOutOfRange {
                    path: String::new(),
                    value,
                });
            } else {
                // `serde_json::Number` is `i64 | u64 | f64`; one of the
                // three accessors above must have matched. Surface as the
                // same recoverable error rather than panicking if the
                // representation ever grows new variants.
                return Err(Error::IntegerOutOfRange {
                    path: String::new(),
                    value: 0,
                });
            }
        }
        Json::String(s) => w.value(s.as_str())?,
        Json::Array(arr) => {
            w.open_array()?;
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    w.val_sep()?;
                    w.space()?;
                }
                write_inline(w, item, placeholder)?;
            }
            w.close_array()?;
        }
        Json::Object(obj) => {
            // Inline table form: { k = v, k = v }
            w.open_inline_table()?;
            for (i, (k, val)) in obj.iter().enumerate() {
                if i > 0 {
                    w.val_sep()?;
                }
                w.space()?;
                w.key(k.as_str())?;
                w.space()?;
                w.keyval_sep()?;
                w.space()?;
                write_inline(w, val, placeholder)?;
            }
            if !obj.is_empty() {
                w.space()?;
            }
            w.close_inline_table()?;
        }
    }
    Ok(())
}

// ============================================================================
// Document-level emission: separate inline keys from sub-tables and
// arrays-of-tables, emit proper [section] and [[array.of.tables]] headers.
// ============================================================================

/// Returns true if the array is non-empty and every element is a JSON Object.
/// Such arrays are emitted as `[[path]]` array-of-tables sections.
fn is_array_of_tables(arr: &[Json]) -> bool {
    !arr.is_empty() && arr.iter().all(|v| matches!(v, Json::Object(_)))
}

fn write_table<W: TomlWrite>(
    w: &mut W,
    path: &[&str],
    obj: &serde_json::Map<String, Json>,
    placeholder: &str,
) -> Result<()> {
    // Pass 1: emit inline keys (scalars, non-table-arrays, inline objects).
    // Pass 2: emit sub-tables and arrays-of-tables as their own sections.
    let mut sub_tables: Vec<(&str, &serde_json::Map<String, Json>)> = Vec::new();
    // Store the typed maps directly so the array-of-tables emission loop
    // never has to re-pattern-match each element. `is_array_of_tables`
    // already proved every element is an Object; encode that proof in the
    // type instead of trusting it via `unreachable!`.
    let mut sub_aots: Vec<(&str, Vec<&serde_json::Map<String, Json>>)> = Vec::new();

    for (k, v) in obj {
        match v {
            Json::Object(child) => sub_tables.push((k, child)),
            Json::Array(arr) if is_array_of_tables(arr) => {
                let tables: Vec<&serde_json::Map<String, Json>> = arr
                    .iter()
                    .filter_map(|item| match item {
                        Json::Object(t) => Some(t),
                        _ => None,
                    })
                    .collect();
                sub_aots.push((k, tables));
            }
            _ => {
                w.key(k.as_str())?;
                w.space()?;
                w.keyval_sep()?;
                w.space()?;
                write_inline(w, v, placeholder)?;
                w.newline()?;
            }
        }
    }

    // Sub-tables.
    for (k, child) in sub_tables {
        w.newline()?;
        w.open_table_header()?;
        for p in path {
            w.key(*p)?;
            w.key_sep()?;
        }
        w.key(k)?;
        w.close_table_header()?;
        w.newline()?;

        let mut new_path: Vec<&str> = path.to_vec();
        new_path.push(k);
        write_table(w, &new_path, child, placeholder)?;
    }

    // Arrays of tables.
    for (k, tables) in sub_aots {
        for table in tables {
            w.newline()?;
            w.open_array_of_tables_header()?;
            for p in path {
                w.key(*p)?;
                w.key_sep()?;
            }
            w.key(k)?;
            w.close_array_of_tables_header()?;
            w.newline()?;

            let mut new_path: Vec<&str> = path.to_vec();
            new_path.push(k);
            write_table(w, &new_path, table, placeholder)?;
        }
    }

    Ok(())
}
