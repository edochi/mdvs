use datafusion::arrow::datatypes::{DataType, Field, Fields};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Recursive type of a frontmatter field, inferred from YAML values.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldType {
    /// YAML `true` / `false`.
    Boolean,
    /// YAML integer (i64/u64).
    Integer,
    /// YAML float (f64). Integer+Float widens to Float.
    Float,
    /// YAML string. Top type in the widening hierarchy — incompatible types widen here.
    String,
    /// YAML sequence with a uniform element type.
    Array(Box<FieldType>),
    /// YAML mapping with named sub-fields.
    Object(BTreeMap<std::string::String, FieldType>),
}

impl FieldType {
    /// Widen two field types into their least upper bound.
    ///
    /// Symmetric: `from_widen(A, B) == from_widen(B, A)`. Rules: same types stay,
    /// `Integer + Float → Float`, mismatched scalars → `String`, arrays
    /// widen inner types, objects merge keys (widening shared keys).
    pub fn from_widen(a: Self, b: Self) -> Self {
        if a == b {
            return a;
        }
        match (a, b) {
            (FieldType::Integer, FieldType::Float) | (FieldType::Float, FieldType::Integer) => {
                FieldType::Float
            }
            (FieldType::Array(x), FieldType::Array(y)) => {
                FieldType::Array(Box::new(Self::from_widen(*x, *y)))
            }
            (FieldType::Object(a), FieldType::Object(b)) => {
                let mut merged = BTreeMap::new();
                for (k, v) in &a {
                    if let Some(bv) = b.get(k) {
                        merged.insert(k.clone(), Self::from_widen(v.clone(), bv.clone()));
                    } else {
                        merged.insert(k.clone(), v.clone());
                    }
                }
                for (k, v) in &b {
                    if !a.contains_key(k) {
                        merged.insert(k.clone(), v.clone());
                    }
                }
                FieldType::Object(merged)
            }
            _ => FieldType::String,
        }
    }
}

impl From<&Value> for FieldType {
    fn from(value: &Value) -> Self {
        match value {
            Value::Bool(_) => FieldType::Boolean,
            Value::Number(n) => {
                if n.is_i64() || n.is_u64() {
                    FieldType::Integer
                } else {
                    FieldType::Float
                }
            }
            Value::String(_) => FieldType::String,
            Value::Array(arr) => {
                if arr.is_empty() {
                    FieldType::Array(Box::new(FieldType::String))
                } else {
                    let mut t = FieldType::from(&arr[0]);
                    for v in &arr[1..] {
                        t = Self::from_widen(t, FieldType::from(v));
                    }
                    FieldType::Array(Box::new(t))
                }
            }
            Value::Object(map) => {
                let fields = map
                    .iter()
                    .map(|(k, v)| (k.clone(), FieldType::from(v)))
                    .collect();
                FieldType::Object(fields)
            }
            Value::Null => FieldType::String,
        }
    }
}

impl From<&FieldType> for DataType {
    fn from(ft: &FieldType) -> Self {
        match ft {
            FieldType::Boolean => DataType::Boolean,
            FieldType::Integer => DataType::Int64,
            FieldType::Float => DataType::Float64,
            FieldType::String => DataType::Utf8,
            FieldType::Array(inner) => {
                let inner_dt: DataType = inner.as_ref().into();
                DataType::List(Arc::new(Field::new("item", inner_dt, true)))
            }
            FieldType::Object(fields) => {
                let arrow_fields: Vec<Field> = fields
                    .iter()
                    .map(|(name, ft)| Field::new(name, ft.into(), true))
                    .collect();
                DataType::Struct(Fields::from(arrow_fields))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Widening tests ---

    #[test]
    fn widen_same_type() {
        assert_eq!(
            FieldType::from_widen(FieldType::Integer, FieldType::Integer),
            FieldType::Integer
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Boolean, FieldType::Boolean),
            FieldType::Boolean
        );
        assert_eq!(
            FieldType::from_widen(FieldType::String, FieldType::String),
            FieldType::String
        );
    }

    #[test]
    fn widen_integer_float() {
        assert_eq!(
            FieldType::from_widen(FieldType::Integer, FieldType::Float),
            FieldType::Float
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Float, FieldType::Integer),
            FieldType::Float
        );
    }

    #[test]
    fn widen_incompatible_scalars_to_string() {
        assert_eq!(
            FieldType::from_widen(FieldType::Boolean, FieldType::Integer),
            FieldType::String
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Boolean, FieldType::String),
            FieldType::String
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Boolean, FieldType::Float),
            FieldType::String
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Integer, FieldType::String),
            FieldType::String
        );
        assert_eq!(
            FieldType::from_widen(FieldType::Float, FieldType::String),
            FieldType::String
        );
    }

    #[test]
    fn widen_array_inner() {
        assert_eq!(
            FieldType::from_widen(
                FieldType::Array(Box::new(FieldType::Integer)),
                FieldType::Array(Box::new(FieldType::Float)),
            ),
            FieldType::Array(Box::new(FieldType::Float)),
        );
        assert_eq!(
            FieldType::from_widen(
                FieldType::Array(Box::new(FieldType::Integer)),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            FieldType::Array(Box::new(FieldType::String)),
        );
    }

    #[test]
    fn widen_array_plus_scalar_to_string() {
        assert_eq!(
            FieldType::from_widen(
                FieldType::Array(Box::new(FieldType::String)),
                FieldType::Integer,
            ),
            FieldType::String,
        );
    }

    #[test]
    fn widen_object_merge_shared_and_unique_keys() {
        let obj_a = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("count".into(), FieldType::Integer),
        ]));
        let obj_b = FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
        ]));
        assert_eq!(
            FieldType::from_widen(obj_a, obj_b),
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("count".into(), FieldType::Integer),
                ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
            ])),
        );
    }

    #[test]
    fn widen_object_shared_key_widened() {
        let obj_c = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Integer)]));
        let obj_d = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Float)]));
        assert_eq!(
            FieldType::from_widen(obj_c, obj_d),
            FieldType::Object(BTreeMap::from([("val".into(), FieldType::Float)])),
        );
    }

    #[test]
    fn widen_object_plus_array_to_string() {
        assert_eq!(
            FieldType::from_widen(
                FieldType::Object(BTreeMap::new()),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            FieldType::String,
        );
    }

    #[test]
    fn widen_object_disjoint_keys() {
        let obj_x = FieldType::Object(BTreeMap::from([
            ("A".into(), FieldType::Integer),
            ("B".into(), FieldType::String),
        ]));
        let obj_y = FieldType::Object(BTreeMap::from([
            ("C".into(), FieldType::Boolean),
            ("D".into(), FieldType::Integer),
        ]));
        assert_eq!(
            FieldType::from_widen(obj_x, obj_y),
            FieldType::Object(BTreeMap::from([
                ("A".into(), FieldType::Integer),
                ("B".into(), FieldType::String),
                ("C".into(), FieldType::Boolean),
                ("D".into(), FieldType::Integer),
            ])),
        );
    }

    #[test]
    fn widen_object_three_way_chained() {
        let o1 = FieldType::Object(BTreeMap::from([("x".into(), FieldType::Integer)]));
        let o2 = FieldType::Object(BTreeMap::from([("y".into(), FieldType::Boolean)]));
        let o3 = FieldType::Object(BTreeMap::from([
            ("x".into(), FieldType::Float),
            ("z".into(), FieldType::String),
        ]));
        assert_eq!(
            FieldType::from_widen(FieldType::from_widen(o1, o2), o3),
            FieldType::Object(BTreeMap::from([
                ("x".into(), FieldType::Float),
                ("y".into(), FieldType::Boolean),
                ("z".into(), FieldType::String),
            ])),
        );
    }

    #[test]
    fn widen_nested_object() {
        let nested_a = FieldType::Object(BTreeMap::from([(
            "meta".into(),
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("version".into(), FieldType::Integer),
            ])),
        )]));
        let nested_b = FieldType::Object(BTreeMap::from([(
            "meta".into(),
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("license".into(), FieldType::String),
                ("version".into(), FieldType::Float),
            ])),
        )]));
        assert_eq!(
            FieldType::from_widen(nested_a, nested_b),
            FieldType::Object(BTreeMap::from([(
                "meta".into(),
                FieldType::Object(BTreeMap::from([
                    ("author".into(), FieldType::String),
                    ("license".into(), FieldType::String),
                    ("version".into(), FieldType::Float),
                ])),
            )])),
        );
    }

    #[test]
    fn widen_object_shared_key_incompatible() {
        let oa = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Boolean)]));
        let ob = FieldType::Object(BTreeMap::from([(
            "val".into(),
            FieldType::Array(Box::new(FieldType::Integer)),
        )]));
        assert_eq!(
            FieldType::from_widen(oa, ob),
            FieldType::Object(BTreeMap::from([("val".into(), FieldType::String)])),
        );
    }

    #[test]
    fn widen_array_of_arrays() {
        assert_eq!(
            FieldType::from_widen(
                FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Integer)))),
                FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float)))),
            ),
            FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float)))),
        );
    }

    #[test]
    fn widen_array_of_objects() {
        let arr_obj_a = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([(
            "id".into(),
            FieldType::Integer,
        )]))));
        let arr_obj_b = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            ("id".into(), FieldType::Integer),
            ("label".into(), FieldType::String),
        ]))));
        assert_eq!(
            FieldType::from_widen(arr_obj_a, arr_obj_b),
            FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
                ("id".into(), FieldType::Integer),
                ("label".into(), FieldType::String),
            ])))),
        );
    }

    #[test]
    fn widen_symmetry() {
        let pairs: Vec<(FieldType, FieldType)> = vec![
            (FieldType::Integer, FieldType::Float),
            (FieldType::Boolean, FieldType::Integer),
            (FieldType::Boolean, FieldType::Float),
            (FieldType::Boolean, FieldType::String),
            (FieldType::Integer, FieldType::String),
            (FieldType::Float, FieldType::String),
            (
                FieldType::Array(Box::new(FieldType::Integer)),
                FieldType::Boolean,
            ),
            (
                FieldType::Array(Box::new(FieldType::Integer)),
                FieldType::Array(Box::new(FieldType::Float)),
            ),
        ];
        for (a, b) in &pairs {
            assert_eq!(
                FieldType::from_widen(a.clone(), b.clone()),
                FieldType::from_widen(b.clone(), a.clone()),
                "symmetry failed for {:?} and {:?}",
                a,
                b,
            );
        }
    }

    // --- From<&Value> tests ---

    #[test]
    fn from_value_mixed_array() {
        let mixed: Value = serde_json::json!([1, "a", true]);
        assert_eq!(
            FieldType::from(&mixed),
            FieldType::Array(Box::new(FieldType::String)),
        );
    }

    #[test]
    fn from_value_homogeneous_arrays() {
        let ints: Value = serde_json::json!([1, 2, 3]);
        assert_eq!(
            FieldType::from(&ints),
            FieldType::Array(Box::new(FieldType::Integer)),
        );

        let nums: Value = serde_json::json!([1, 2.5]);
        assert_eq!(
            FieldType::from(&nums),
            FieldType::Array(Box::new(FieldType::Float)),
        );
    }

    #[test]
    fn from_value_object() {
        let obj: Value = serde_json::json!({"author": "me", "count": 3});
        assert_eq!(
            FieldType::from(&obj),
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("count".into(), FieldType::Integer),
            ])),
        );
    }

    #[test]
    fn from_value_nested_object() {
        let deep: Value = serde_json::json!({
            "meta": {"author": "me", "version": 2},
            "tags": ["rust", "arrow"]
        });
        assert_eq!(
            FieldType::from(&deep),
            FieldType::Object(BTreeMap::from([
                (
                    "meta".into(),
                    FieldType::Object(BTreeMap::from([
                        ("author".into(), FieldType::String),
                        ("version".into(), FieldType::Integer),
                    ]))
                ),
                ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
            ])),
        );
    }

    #[test]
    fn from_value_array_of_objects() {
        let arr: Value = serde_json::json!([
            {"name": "Alice", "age": 30},
            {"name": "Bob", "age": 25}
        ]);
        assert_eq!(
            FieldType::from(&arr),
            FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
                ("age".into(), FieldType::Integer),
                ("name".into(), FieldType::String),
            ])))),
        );
    }

    #[test]
    fn from_value_array_of_objects_different_keys() {
        let arr: Value = serde_json::json!([
            {"name": "Alice"},
            {"name": "Bob", "email": "bob@x.com"}
        ]);
        assert_eq!(
            FieldType::from(&arr),
            FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
                ("email".into(), FieldType::String),
                ("name".into(), FieldType::String),
            ])))),
        );
    }

    #[test]
    fn from_value_array_of_objects_conflicting_types() {
        let arr: Value = serde_json::json!([
            {"val": 42},
            {"val": "hello"}
        ]);
        assert_eq!(
            FieldType::from(&arr),
            FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([(
                "val".into(),
                FieldType::String
            ),])))),
        );
    }

    #[test]
    fn from_value_empty_array() {
        let empty: Value = serde_json::json!([]);
        assert_eq!(
            FieldType::from(&empty),
            FieldType::Array(Box::new(FieldType::String)),
        );
    }

    #[test]
    fn from_value_null() {
        assert_eq!(FieldType::from(&Value::Null), FieldType::String);
    }

    #[test]
    fn multi_file_widening() {
        let files: Vec<Value> = vec![
            serde_json::json!({"title": "Hello", "count": 42, "meta": {"a": 1}}),
            serde_json::json!({"title": "World", "draft": true, "meta": {"b": "x"}}),
            serde_json::json!({"count": 3.5, "tags": ["go"], "meta": {"a": 2.0, "c": true}}),
        ];
        let mut field_types: BTreeMap<std::string::String, FieldType> = BTreeMap::new();
        for file in &files {
            if let Value::Object(map) = file {
                for (key, val) in map {
                    let ft = FieldType::from(val);
                    field_types
                        .entry(key.clone())
                        .and_modify(|existing| {
                            *existing = FieldType::from_widen(existing.clone(), ft.clone())
                        })
                        .or_insert(ft);
                }
            }
        }
        assert_eq!(field_types["title"], FieldType::String);
        assert_eq!(field_types["count"], FieldType::Float);
        assert_eq!(field_types["draft"], FieldType::Boolean);
        assert_eq!(
            field_types["tags"],
            FieldType::Array(Box::new(FieldType::String))
        );
        assert_eq!(
            field_types["meta"],
            FieldType::Object(BTreeMap::from([
                ("a".into(), FieldType::Float),
                ("b".into(), FieldType::String),
                ("c".into(), FieldType::Boolean),
            ])),
        );
    }

    // --- Arrow type mapping tests ---

    #[test]
    fn arrow_type_primitives() {
        let cases: Vec<(FieldType, DataType)> = vec![
            (FieldType::Boolean, DataType::Boolean),
            (FieldType::Integer, DataType::Int64),
            (FieldType::Float, DataType::Float64),
            (FieldType::String, DataType::Utf8),
        ];
        for (ft, expected) in &cases {
            let got: DataType = ft.into();
            assert_eq!(&got, expected);
        }
    }

    #[test]
    fn arrow_type_array() {
        let ft = FieldType::Array(Box::new(FieldType::String));
        let got: DataType = (&ft).into();
        assert_eq!(
            got,
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        );

        let ft2 = FieldType::Array(Box::new(FieldType::Integer));
        let got2: DataType = (&ft2).into();
        assert_eq!(
            got2,
            DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
        );
    }

    #[test]
    fn arrow_type_object() {
        let ft = FieldType::Object(BTreeMap::from([
            ("name".into(), FieldType::String),
            ("score".into(), FieldType::Float),
        ]));
        let got: DataType = (&ft).into();
        assert_eq!(
            got,
            DataType::Struct(Fields::from(vec![
                Field::new("name", DataType::Utf8, true),
                Field::new("score", DataType::Float64, true),
            ])),
        );
    }
}
