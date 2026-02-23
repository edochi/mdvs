use std::fmt;

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    String,
    #[serde(rename = "string[]")]
    StringArray,
    Date,
    Boolean,
    Integer,
    Float,
    Enum,
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FieldType::String => write!(f, "string"),
            FieldType::StringArray => write!(f, "string[]"),
            FieldType::Date => write!(f, "date"),
            FieldType::Boolean => write!(f, "boolean"),
            FieldType::Integer => write!(f, "integer"),
            FieldType::Float => write!(f, "float"),
            FieldType::Enum => write!(f, "enum"),
        }
    }
}

impl FieldType {
    pub fn sql_type(&self) -> &'static str {
        match self {
            FieldType::String => "VARCHAR",
            FieldType::StringArray => "VARCHAR[]",
            FieldType::Date => "DATE",
            FieldType::Boolean => "BOOLEAN",
            FieldType::Integer => "BIGINT",
            FieldType::Float => "DOUBLE",
            FieldType::Enum => "VARCHAR",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_lowercase() {
        assert_eq!(FieldType::String.to_string(), "string");
        assert_eq!(FieldType::StringArray.to_string(), "string[]");
        assert_eq!(FieldType::Date.to_string(), "date");
        assert_eq!(FieldType::Boolean.to_string(), "boolean");
        assert_eq!(FieldType::Integer.to_string(), "integer");
        assert_eq!(FieldType::Float.to_string(), "float");
        assert_eq!(FieldType::Enum.to_string(), "enum");
    }

    #[test]
    fn sql_types() {
        assert_eq!(FieldType::String.sql_type(), "VARCHAR");
        assert_eq!(FieldType::Enum.sql_type(), "VARCHAR");
        assert_eq!(FieldType::StringArray.sql_type(), "VARCHAR[]");
    }

    #[test]
    fn deserialize_from_toml() {
        #[derive(Deserialize)]
        struct Wrapper {
            #[serde(rename = "type")]
            field_type: FieldType,
        }
        let w: Wrapper = toml::from_str(r#"type = "string""#).unwrap();
        assert_eq!(w.field_type, FieldType::String);

        let w: Wrapper = toml::from_str(r#"type = "string[]""#).unwrap();
        assert_eq!(w.field_type, FieldType::StringArray);

        let w: Wrapper = toml::from_str(r#"type = "enum""#).unwrap();
        assert_eq!(w.field_type, FieldType::Enum);
    }
}
