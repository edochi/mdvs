use crate::discover::field_type::FieldType;
use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Serde-friendly representation of `FieldType` for TOML.
///
/// On-disk form is a function-style string (TODO-0155):
///
/// - `type = "String"`
/// - `type = "Array(String)"`
///
/// Grammar accepted by [`FieldTypeSerde::parse`]:
///
/// ```text
/// Type   := Scalar | "Array(" Scalar ")"
/// Scalar := "String" | "Integer" | "Float" | "Boolean"
/// ```
///
/// `Object{...}`, `Array(Object{...})`, `Array(Array(...))`, and `@Name`
/// references are rejected at parse with a column offset. Top-level
/// `Object` is expressed via dotted-name leaves (Wave C / TODO-0097);
/// `Array(Object{...})` has no v0 representation and is tracked in
/// [TODO-0156](../../../docs/spec/todos/TODO-0156.md). Use parallel
/// scalar arrays as a workaround.
///
/// The enum variants retain `Object` and the recursive `Array` shape so
/// programmatic constructions (e.g. tests, JSON Schema translation) can
/// still describe these shapes; `Display` renders them for diagnostic
/// output. The parser is the gate that enforces what's legal on disk.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldTypeSerde {
    /// A primitive type name: `"Boolean"`, `"Integer"`, `"Float"`, or `"String"`.
    Scalar(String),
    /// An array type with an inner element type.
    Array {
        /// Inner element type.
        array: Box<FieldTypeSerde>,
    },
    /// An object type with named sub-fields.
    Object {
        /// Map of sub-field names to their types.
        object: BTreeMap<String, FieldTypeSerde>,
    },
}

/// Error returned by [`FieldTypeSerde::parse`].
///
/// Holds a human-readable message plus the 1-based column offset where
/// the offending token starts. Surfaces through serde's `de::Error::custom`
/// during TOML deserialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Human-readable error message.
    pub message: String,
    /// 1-based column offset of the offending token.
    pub column: usize,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at column {}: {}", self.column, self.message)
    }
}

impl std::error::Error for ParseError {}

impl FieldTypeSerde {
    /// Parse a function-style type expression into a `FieldTypeSerde`.
    ///
    /// Accepts the v0 grammar (Scalar | Array(Scalar)). Rejects every
    /// other shape with a clear error pointing at the offending column.
    pub fn parse(s: &str) -> Result<Self, ParseError> {
        let mut p = Parser::new(s);
        let ty = p.parse_type()?;
        p.skip_ws();
        if !p.at_end() {
            return Err(ParseError {
                message: "unexpected content after type expression".into(),
                column: p.column(),
            });
        }
        Ok(ty)
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser {
            input: s.as_bytes(),
            pos: 0,
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn column(&self) -> usize {
        self.pos + 1
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_ident(&mut self) -> Result<&'a str, ParseError> {
        self.skip_ws();
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() || c == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return Err(ParseError {
                message: "expected a type name (String, Integer, Float, Boolean, or Array(...))"
                    .into(),
                column: start + 1,
            });
        }
        Ok(std::str::from_utf8(&self.input[start..self.pos]).expect("ascii-only by construction"))
    }

    fn expect_char(&mut self, c: u8, ctx: &str) -> Result<(), ParseError> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(ParseError {
                message: format!("expected '{}' {ctx}", c as char),
                column: self.pos + 1,
            })
        }
    }

    fn parse_type(&mut self) -> Result<FieldTypeSerde, ParseError> {
        self.skip_ws();
        if self.peek() == Some(b'@') {
            return Err(ParseError {
                message: "reference syntax (@Name) is not supported in v0 (see TODO-0156)".into(),
                column: self.pos + 1,
            });
        }
        let ident_start = self.pos;
        let ident = self.parse_ident()?;
        match ident {
            "String" | "Integer" | "Float" | "Boolean" => {
                Ok(FieldTypeSerde::Scalar(ident.to_string()))
            }
            "Array" => {
                self.expect_char(b'(', "after `Array`")?;
                let inner = self.parse_array_inner()?;
                self.expect_char(b')', "to close `Array(...)`")?;
                Ok(FieldTypeSerde::Array {
                    array: Box::new(inner),
                })
            }
            "Object" => Err(ParseError {
                message: "Object types are not supported on disk (Wave C). \
                          Express nested structure via dotted-name leaves \
                          (e.g. `calibration.baseline.wavelength`)"
                    .into(),
                column: ident_start + 1,
            }),
            other => Err(ParseError {
                message: format!(
                    "unknown type `{other}`. \
                     Expected one of String, Integer, Float, Boolean, or Array(...)"
                ),
                column: ident_start + 1,
            }),
        }
    }

    fn parse_array_inner(&mut self) -> Result<FieldTypeSerde, ParseError> {
        self.skip_ws();
        if self.peek() == Some(b'@') {
            return Err(ParseError {
                message: "reference syntax (@Name) is not supported in v0 (see TODO-0156)".into(),
                column: self.pos + 1,
            });
        }
        let inner_start = self.pos;
        let ident = self.parse_ident()?;
        match ident {
            "String" | "Integer" | "Float" | "Boolean" => {
                Ok(FieldTypeSerde::Scalar(ident.to_string()))
            }
            "Array" => Err(ParseError {
                message: "Array of Array is not supported".into(),
                column: inner_start + 1,
            }),
            "Object" => Err(ParseError {
                message: "Array(Object{...}) is not supported in v0. \
                          Consider parallel scalar arrays (see TODO-0156)"
                    .into(),
                column: inner_start + 1,
            }),
            other => Err(ParseError {
                message: format!("unknown type `{other}` inside Array(...)"),
                column: inner_start + 1,
            }),
        }
    }
}

impl Serialize for FieldTypeSerde {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for FieldTypeSerde {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = FieldTypeSerde;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a type expression string like \"Array(String)\"")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Self::Value, E> {
                FieldTypeSerde::parse(s).map_err(E::custom)
            }
            fn visit_string<E: de::Error>(self, s: String) -> Result<Self::Value, E> {
                FieldTypeSerde::parse(&s).map_err(E::custom)
            }
        }
        d.deserialize_str(V)
    }
}

impl From<&FieldType> for FieldTypeSerde {
    fn from(ft: &FieldType) -> Self {
        match ft {
            FieldType::Boolean => FieldTypeSerde::Scalar("Boolean".into()),
            FieldType::Integer => FieldTypeSerde::Scalar("Integer".into()),
            FieldType::Float => FieldTypeSerde::Scalar("Float".into()),
            FieldType::String => FieldTypeSerde::Scalar("String".into()),
            FieldType::Array(inner) => FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::from(inner.as_ref())),
            },
            FieldType::Object(fields) => FieldTypeSerde::Object {
                object: fields
                    .iter()
                    .map(|(k, v)| (k.clone(), FieldTypeSerde::from(v)))
                    .collect(),
            },
        }
    }
}

impl TryFrom<&FieldTypeSerde> for FieldType {
    type Error = String;

    fn try_from(s: &FieldTypeSerde) -> Result<Self, Self::Error> {
        match s {
            FieldTypeSerde::Scalar(name) => match name.as_str() {
                "Boolean" => Ok(FieldType::Boolean),
                "Integer" => Ok(FieldType::Integer),
                "Float" => Ok(FieldType::Float),
                "String" => Ok(FieldType::String),
                other => Err(format!("unknown type: {other}")),
            },
            FieldTypeSerde::Array { array } => {
                let inner = FieldType::try_from(array.as_ref())?;
                Ok(FieldType::Array(Box::new(inner)))
            }
            FieldTypeSerde::Object { object } => {
                let mut fields = BTreeMap::new();
                for (k, v) in object {
                    fields.insert(k.clone(), FieldType::try_from(v)?);
                }
                Ok(FieldType::Object(fields))
            }
        }
    }
}

/// Configuration for file scanning (`[scan]` in `mdvs.toml`).
#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScanConfig {
    /// Glob pattern for matching markdown files.
    pub glob: String,
    /// Whether to include files without YAML frontmatter.
    pub include_bare_files: bool,
    /// Skip reading `.gitignore` patterns during scan.
    #[serde(default)]
    pub skip_gitignore: bool,
}

/// Embedding model identity (`[embedding_model]` in `mdvs.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EmbeddingModelConfig {
    /// Provider name (e.g. `"model2vec"`).
    #[serde(default = "default_provider")]
    pub provider: String,
    /// HuggingFace model ID (e.g. `"minishlab/potion-base-8M"`).
    pub name: String,
    /// Pinned revision (commit SHA).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
}

fn default_provider() -> String {
    "model2vec".to_string()
}

/// Chunking settings (`[chunking]` in `mdvs.toml`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ChunkingConfig {
    /// Maximum chunk size in characters.
    pub max_chunk_size: usize,
}

impl fmt::Display for FieldTypeSerde {
    /// Function-style rendering (TODO-0096, landed as part of TODO-0097 step 7):
    /// `Array(String)` rather than `String[]`, `Object{k: v, ...}` rather than
    /// the bare `{k: v, ...}`. The "Array(...)"/"Object{...}" prefixes make
    /// nested compositions read uniformly:
    ///
    /// - `Array(String)`
    /// - `Array(Array(Integer))`
    /// - `Array(Object{time: String, value: Float})`
    ///
    /// Top-level `Object` doesn't appear in output for valid configs after
    /// [`crate::schema::config::MdvsToml::validate`]'s invariant 6, but the
    /// `Object` arm here remains in use for `Array(Object{...})` rendering.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldTypeSerde::Scalar(name) => write!(f, "{name}"),
            FieldTypeSerde::Array { array } => write!(f, "Array({array})"),
            FieldTypeSerde::Object { object } => {
                write!(f, "Object{{")?;
                for (i, (k, v)) in object.iter().enumerate() {
                    if i > 0 {
                        write!(f, ", ")?;
                    }
                    write!(f, "{k}: {v}")?;
                }
                write!(f, "}}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wrapper for testing FieldTypeSerde in isolation (TOML needs a root table).
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TypeWrapper {
        #[serde(rename = "type")]
        field_type: FieldTypeSerde,
    }

    fn wrap(ft: &FieldType) -> TypeWrapper {
        TypeWrapper {
            field_type: FieldTypeSerde::from(ft),
        }
    }

    #[test]
    fn scalar_roundtrip() {
        let types = vec![
            FieldType::Boolean,
            FieldType::Integer,
            FieldType::Float,
            FieldType::String,
        ];
        for ft in &types {
            let w = wrap(ft);
            let toml_str = toml::to_string(&w).unwrap();
            let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
            let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
            assert_eq!(&roundtripped, ft);
        }
    }

    #[test]
    fn array_string_roundtrip() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains(r#"type = "Array(String)""#));
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
    }

    #[test]
    fn unknown_scalar_type_error() {
        let bad = FieldTypeSerde::Scalar("Date".into());
        let result = FieldType::try_from(&bad);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown type"));
    }

    // --- Display tests ----------------------------------------------------
    //
    // Display is a *superset* of parse: it still renders Object{...} and
    // Array(Object{...}) and Array(Array(...)) for diagnostic output, even
    // though the parser rejects those shapes. Pin this asymmetry.

    #[test]
    fn display_scalar() {
        let ft = FieldTypeSerde::Scalar("String".into());
        assert_eq!(ft.to_string(), "String");
    }

    #[test]
    fn display_array() {
        let ft = FieldTypeSerde::Array {
            array: Box::new(FieldTypeSerde::Scalar("String".into())),
        };
        assert_eq!(ft.to_string(), "Array(String)");
    }

    #[test]
    fn display_nested_array() {
        let ft = FieldTypeSerde::Array {
            array: Box::new(FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Integer".into())),
            }),
        };
        assert_eq!(ft.to_string(), "Array(Array(Integer))");
    }

    #[test]
    fn display_object_for_diagnostics() {
        let ft = FieldTypeSerde::Object {
            object: BTreeMap::from([
                ("author".into(), FieldTypeSerde::Scalar("String".into())),
                ("count".into(), FieldTypeSerde::Scalar("Integer".into())),
            ]),
        };
        assert_eq!(ft.to_string(), "Object{author: String, count: Integer}");
        // Asymmetry: Display renders, parse rejects.
        assert!(FieldTypeSerde::parse(&ft.to_string()).is_err());
    }

    #[test]
    fn display_array_of_object_for_diagnostics() {
        let ft = FieldTypeSerde::Array {
            array: Box::new(FieldTypeSerde::Object {
                object: BTreeMap::from([
                    ("time".into(), FieldTypeSerde::Scalar("String".into())),
                    ("value".into(), FieldTypeSerde::Scalar("Float".into())),
                ]),
            }),
        };
        assert_eq!(ft.to_string(), "Array(Object{time: String, value: Float})");
        // Asymmetry: Display renders, parse rejects.
        assert!(FieldTypeSerde::parse(&ft.to_string()).is_err());
    }

    // --- Parser tests -----------------------------------------------------

    #[test]
    fn parse_scalar_all_four() {
        for name in &["String", "Integer", "Float", "Boolean"] {
            let ft = FieldTypeSerde::parse(name).unwrap();
            assert_eq!(ft, FieldTypeSerde::Scalar((*name).into()));
        }
    }

    #[test]
    fn parse_array_of_each_scalar() {
        for name in &["String", "Integer", "Float", "Boolean"] {
            let s = format!("Array({name})");
            let ft = FieldTypeSerde::parse(&s).unwrap();
            assert_eq!(
                ft,
                FieldTypeSerde::Array {
                    array: Box::new(FieldTypeSerde::Scalar((*name).into())),
                }
            );
        }
    }

    #[test]
    fn parse_whitespace_tolerant() {
        let ft = FieldTypeSerde::parse("  Array(  String  )  ").unwrap();
        assert_eq!(
            ft,
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            }
        );
    }

    #[test]
    fn parse_rejects_array_of_object_points_at_object() {
        let err = FieldTypeSerde::parse("Array(Object{x: String})").unwrap_err();
        assert!(err.message.contains("Array(Object"));
        assert!(err.message.contains("TODO-0156"));
        // 'Object' starts at byte 7 → column 7 (1-based).
        assert_eq!(err.column, 7);
    }

    #[test]
    fn parse_rejects_array_of_array() {
        let err = FieldTypeSerde::parse("Array(Array(String))").unwrap_err();
        assert!(err.message.contains("Array of Array"));
        assert_eq!(err.column, 7);
    }

    #[test]
    fn parse_rejects_object_top_level() {
        let err = FieldTypeSerde::parse("Object{x: String}").unwrap_err();
        assert!(err.message.contains("Object types are not supported"));
        assert!(err.message.contains("dotted-name leaves"));
        assert_eq!(err.column, 1);
    }

    #[test]
    fn parse_rejects_at_ref_mentions_todo_0156() {
        let err = FieldTypeSerde::parse("@SensorReading").unwrap_err();
        assert!(err.message.contains("@Name"));
        assert!(err.message.contains("TODO-0156"));
        assert_eq!(err.column, 1);
    }

    #[test]
    fn parse_rejects_at_ref_inside_array() {
        let err = FieldTypeSerde::parse("Array(@SensorReading)").unwrap_err();
        assert!(err.message.contains("@Name"));
        assert!(err.message.contains("TODO-0156"));
        assert_eq!(err.column, 7);
    }

    #[test]
    fn parse_rejects_unknown_scalar() {
        let err = FieldTypeSerde::parse("Date").unwrap_err();
        assert!(err.message.contains("unknown type `Date`"));
        assert_eq!(err.column, 1);
    }

    #[test]
    fn parse_rejects_trailing_content() {
        let err = FieldTypeSerde::parse("String foo").unwrap_err();
        assert!(err.message.contains("unexpected content"));
    }

    #[test]
    fn parse_rejects_missing_open_paren() {
        let err = FieldTypeSerde::parse("Array String").unwrap_err();
        assert!(err.message.contains("expected '('"));
    }

    #[test]
    fn parse_rejects_missing_close_paren() {
        let err = FieldTypeSerde::parse("Array(String").unwrap_err();
        assert!(err.message.contains("expected ')'"));
    }

    #[test]
    fn parse_rejects_empty_input() {
        let err = FieldTypeSerde::parse("").unwrap_err();
        assert!(err.message.contains("expected a type name"));
    }

    #[test]
    fn parse_rejects_empty_array() {
        let err = FieldTypeSerde::parse("Array()").unwrap_err();
        assert!(err.message.contains("expected a type name"));
    }

    #[test]
    fn roundtrip_parse_display() {
        let cases = vec![
            FieldTypeSerde::Scalar("String".into()),
            FieldTypeSerde::Scalar("Integer".into()),
            FieldTypeSerde::Scalar("Float".into()),
            FieldTypeSerde::Scalar("Boolean".into()),
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            },
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Float".into())),
            },
        ];
        for ft in cases {
            let s = ft.to_string();
            let parsed = FieldTypeSerde::parse(&s).expect(&s);
            assert_eq!(parsed, ft);
        }
    }

    // --- Serde wire-format tests ------------------------------------------

    #[test]
    fn deserialize_function_style_string() {
        let toml_str = r#"type = "Array(String)""#;
        let w: TypeWrapper = toml::from_str(toml_str).unwrap();
        assert_eq!(
            w.field_type,
            FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("String".into())),
            }
        );
    }

    #[test]
    fn deserialize_rejects_old_inline_table() {
        let toml_str = r#"type = { array = "String" }"#;
        let result: Result<TypeWrapper, _> = toml::from_str(toml_str);
        assert!(
            result.is_err(),
            "old inline-table form must fail; got {result:?}"
        );
    }

    #[test]
    fn deserialize_rejects_old_section_table() {
        let toml_str = "[type]\narray = \"String\"\n";
        let result: Result<TypeWrapper, _> = toml::from_str(toml_str);
        assert!(result.is_err());
    }

    #[test]
    fn serialize_emits_function_style_string() {
        let w = TypeWrapper {
            field_type: FieldTypeSerde::Array {
                array: Box::new(FieldTypeSerde::Scalar("Float".into())),
            },
        };
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains(r#"type = "Array(Float)""#));
    }

    #[test]
    fn serialize_emits_bare_scalar() {
        let w = TypeWrapper {
            field_type: FieldTypeSerde::Scalar("Integer".into()),
        };
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains(r#"type = "Integer""#));
    }

    #[test]
    fn model_info_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Wrapper {
            model: EmbeddingModelConfig,
        }
        let w = Wrapper {
            model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: Some("abc123".into()),
            },
        };
        let toml_str = toml::to_string(&w).unwrap();
        let parsed: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, w);
    }

    #[test]
    fn model_info_no_revision() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct Wrapper {
            model: EmbeddingModelConfig,
        }
        let w = Wrapper {
            model: EmbeddingModelConfig {
                provider: "model2vec".into(),
                name: "minishlab/potion-base-8M".into(),
                revision: None,
            },
        };
        let toml_str = toml::to_string(&w).unwrap();
        assert!(!toml_str.contains("revision"));
        let parsed: Wrapper = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed, w);
    }
}
