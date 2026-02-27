use crate::discover::field_type::FieldType;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Serde-friendly representation of FieldType for TOML.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FieldTypeSerde {
    Scalar(String),
    Array { array: Box<FieldTypeSerde> },
    Object { object: BTreeMap<String, FieldTypeSerde> },
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

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct TomlConfig {
    pub glob: String,
    pub include_bare_files: bool,
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
        assert!(toml_str.contains("array"));
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
    }

    #[test]
    fn array_array_float_roundtrip() {
        let ft = FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float))));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
    }

    #[test]
    fn object_roundtrip() {
        let ft = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("version".into(), FieldType::Float),
            ("draft".into(), FieldType::Boolean),
        ]));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
        assert!(toml_str.contains("object"));
        let parsed: TypeWrapper = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed.field_type).unwrap();
        assert_eq!(roundtripped, ft);
    }

    #[test]
    fn object_with_nested_array_roundtrip() {
        let ft = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
        ]));
        let w = wrap(&ft);
        let toml_str = toml::to_string(&w).unwrap();
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

    #[test]
    fn deeply_nested_type_roundtrip() {
        // Array(Object(tags: Array(String), meta: Object(x: Integer)))
        let ft = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            (
                "meta".into(),
                FieldType::Object(BTreeMap::from([("x".into(), FieldType::Integer)])),
            ),
        ]))));
        let serde = FieldTypeSerde::from(&ft);
        let toml_str = toml::to_string(&serde).unwrap();
        let parsed: FieldTypeSerde = toml::from_str(&toml_str).unwrap();
        let roundtripped = FieldType::try_from(&parsed).unwrap();
        assert_eq!(roundtripped, ft);
    }
}
