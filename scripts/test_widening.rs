#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! arrow = "54"
//! serde_json = "1"
//! ```

use arrow::array::*;
use arrow::buffer::OffsetBuffer;
use arrow::datatypes::{DataType, Field, Fields, Schema};
use arrow::record_batch::RecordBatch;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

// --- FieldType enum ---

#[derive(Debug, Clone, PartialEq)]
enum FieldType {
    Boolean,
    Integer,
    Float,
    String,
    Array(Box<FieldType>),
    Object(BTreeMap<std::string::String, FieldType>),
}

// --- Widening ---

fn widen(a: FieldType, b: FieldType) -> FieldType {
    if a == b {
        return a;
    }
    match (a, b) {
        (FieldType::Integer, FieldType::Float) | (FieldType::Float, FieldType::Integer) => {
            FieldType::Float
        }
        (FieldType::Array(x), FieldType::Array(y)) => {
            FieldType::Array(Box::new(widen(*x, *y)))
        }
        (FieldType::Object(a), FieldType::Object(b)) => {
            let mut merged = BTreeMap::new();
            for (k, v) in &a {
                if let Some(bv) = b.get(k) {
                    merged.insert(k.clone(), widen(v.clone(), bv.clone()));
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

// --- Infer FieldType from serde_json::Value ---

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
                        t = widen(t, FieldType::from(v));
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
            Value::Null => FieldType::String, // null → string fallback
        }
    }
}

// --- FieldType → Arrow DataType ---

fn to_arrow_type(ft: &FieldType) -> DataType {
    match ft {
        FieldType::Boolean => DataType::Boolean,
        FieldType::Integer => DataType::Int64,
        FieldType::Float => DataType::Float64,
        FieldType::String => DataType::Utf8,
        FieldType::Array(inner) => {
            DataType::List(Arc::new(Field::new("item", to_arrow_type(inner), true)))
        }
        FieldType::Object(fields) => {
            let arrow_fields: Vec<Field> = fields
                .iter()
                .map(|(name, ft)| Field::new(name, to_arrow_type(ft), true))
                .collect();
            DataType::Struct(Fields::from(arrow_fields))
        }
    }
}

fn main() {
    println!("=== Widening tests ===\n");

    // Basic same type
    assert_eq!(widen(FieldType::Integer, FieldType::Integer), FieldType::Integer);
    println!("  Integer + Integer = Integer  ✓");

    // Integer + Float → Float
    assert_eq!(widen(FieldType::Integer, FieldType::Float), FieldType::Float);
    println!("  Integer + Float   = Float    ✓");

    // Float + Integer → Float (symmetric)
    assert_eq!(widen(FieldType::Float, FieldType::Integer), FieldType::Float);
    println!("  Float + Integer   = Float    ✓");

    // Boolean + Integer → String
    assert_eq!(widen(FieldType::Boolean, FieldType::Integer), FieldType::String);
    println!("  Boolean + Integer = String   ✓");

    // Boolean + String → String
    assert_eq!(widen(FieldType::Boolean, FieldType::String), FieldType::String);
    println!("  Boolean + String  = String   ✓");

    // Array(Integer) + Array(Float) → Array(Float)
    assert_eq!(
        widen(
            FieldType::Array(Box::new(FieldType::Integer)),
            FieldType::Array(Box::new(FieldType::Float))
        ),
        FieldType::Array(Box::new(FieldType::Float))
    );
    println!("  Array(Int) + Array(Float) = Array(Float)  ✓");

    // Array(Integer) + Array(String) → Array(String)
    assert_eq!(
        widen(
            FieldType::Array(Box::new(FieldType::Integer)),
            FieldType::Array(Box::new(FieldType::String))
        ),
        FieldType::Array(Box::new(FieldType::String))
    );
    println!("  Array(Int) + Array(String) = Array(String) ✓");

    // Array + Integer → String
    assert_eq!(
        widen(
            FieldType::Array(Box::new(FieldType::String)),
            FieldType::Integer
        ),
        FieldType::String
    );
    println!("  Array(String) + Integer = String  ✓");

    // Object merge
    let obj_a = FieldType::Object(BTreeMap::from([
        ("author".into(), FieldType::String),
        ("count".into(), FieldType::Integer),
    ]));
    let obj_b = FieldType::Object(BTreeMap::from([
        ("author".into(), FieldType::String),
        ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
    ]));
    let merged = widen(obj_a, obj_b);
    assert_eq!(
        merged,
        FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("count".into(), FieldType::Integer),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
        ]))
    );
    println!("  Object merge: shared + unique keys ✓");

    // Object merge with widening shared key
    let obj_c = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Integer)]));
    let obj_d = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Float)]));
    assert_eq!(
        widen(obj_c, obj_d),
        FieldType::Object(BTreeMap::from([("val".into(), FieldType::Float)]))
    );
    println!("  Object merge: widen shared key (Int+Float→Float) ✓");

    // Object + Array → String
    assert_eq!(
        widen(
            FieldType::Object(BTreeMap::new()),
            FieldType::Array(Box::new(FieldType::String))
        ),
        FieldType::String
    );
    println!("  Object + Array = String  ✓");

    // --- Object: completely disjoint keys expand ---
    let obj_x = FieldType::Object(BTreeMap::from([
        ("A".into(), FieldType::Integer),
        ("B".into(), FieldType::String),
    ]));
    let obj_y = FieldType::Object(BTreeMap::from([
        ("C".into(), FieldType::Boolean),
        ("D".into(), FieldType::Integer),
    ]));
    assert_eq!(
        widen(obj_x, obj_y),
        FieldType::Object(BTreeMap::from([
            ("A".into(), FieldType::Integer),
            ("B".into(), FieldType::String),
            ("C".into(), FieldType::Boolean),
            ("D".into(), FieldType::Integer),
        ]))
    );
    println!("  Object disjoint keys: {{A,B}} + {{C,D}} = {{A,B,C,D}}  ✓");

    // --- Object: 3-way widening (chained) ---
    let o1 = FieldType::Object(BTreeMap::from([("x".into(), FieldType::Integer)]));
    let o2 = FieldType::Object(BTreeMap::from([("y".into(), FieldType::Boolean)]));
    let o3 = FieldType::Object(BTreeMap::from([
        ("x".into(), FieldType::Float),
        ("z".into(), FieldType::String),
    ]));
    let result = widen(widen(o1, o2), o3);
    assert_eq!(
        result,
        FieldType::Object(BTreeMap::from([
            ("x".into(), FieldType::Float),  // Integer widened to Float
            ("y".into(), FieldType::Boolean),
            ("z".into(), FieldType::String),
        ]))
    );
    println!("  Object 3-way: {{x:Int}} + {{y:Bool}} + {{x:Float,z:Str}} = {{x:Float,y:Bool,z:Str}}  ✓");

    // --- Nested object inside object ---
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
            ("version".into(), FieldType::Float), // widens Integer→Float
        ])),
    )]));
    let nested_merged = widen(nested_a, nested_b);
    assert_eq!(
        nested_merged,
        FieldType::Object(BTreeMap::from([(
            "meta".into(),
            FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("license".into(), FieldType::String),
                ("version".into(), FieldType::Float),
            ])),
        )]))
    );
    println!("  Nested object: meta.version Int+Float=Float, meta.license added  ✓");

    // --- Object where shared key has incompatible types → String ---
    let oa = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Boolean)]));
    let ob = FieldType::Object(BTreeMap::from([("val".into(), FieldType::Array(Box::new(FieldType::Integer)))]));
    assert_eq!(
        widen(oa, ob),
        FieldType::Object(BTreeMap::from([("val".into(), FieldType::String)]))
    );
    println!("  Object shared key: Bool + Array(Int) = String  ✓");

    // --- Array of arrays ---
    assert_eq!(
        widen(
            FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Integer)))),
            FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float)))),
        ),
        FieldType::Array(Box::new(FieldType::Array(Box::new(FieldType::Float)))),
    );
    println!("  Array(Array(Int)) + Array(Array(Float)) = Array(Array(Float))  ✓");

    // --- Array of objects ---
    let arr_obj_a = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
        ("id".into(), FieldType::Integer),
    ]))));
    let arr_obj_b = FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
        ("id".into(), FieldType::Integer),
        ("label".into(), FieldType::String),
    ]))));
    assert_eq!(
        widen(arr_obj_a, arr_obj_b),
        FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            ("id".into(), FieldType::Integer),
            ("label".into(), FieldType::String),
        ]))))
    );
    println!("  Array(Object{{id}}) + Array(Object{{id,label}}) = Array(Object{{id,label}})  ✓");

    // --- Symmetry: widen(a,b) == widen(b,a) for all basic combinations ---
    let pairs: Vec<(FieldType, FieldType)> = vec![
        (FieldType::Integer, FieldType::Float),
        (FieldType::Boolean, FieldType::Integer),
        (FieldType::Boolean, FieldType::Float),
        (FieldType::Boolean, FieldType::String),
        (FieldType::Integer, FieldType::String),
        (FieldType::Float, FieldType::String),
        (FieldType::Array(Box::new(FieldType::Integer)), FieldType::Boolean),
        (FieldType::Array(Box::new(FieldType::Integer)), FieldType::Array(Box::new(FieldType::Float))),
    ];
    for (a, b) in &pairs {
        assert_eq!(
            widen(a.clone(), b.clone()),
            widen(b.clone(), a.clone()),
            "symmetry failed for {:?} and {:?}",
            a,
            b
        );
    }
    println!("  Symmetry: widen(a,b) == widen(b,a) for all pairs  ✓");

    println!("\n=== Infer type from JSON values ===\n");

    // Mixed array → Array(String)
    let mixed: Value = serde_json::json!([1, "a", true]);
    assert_eq!(
        FieldType::from(&mixed),
        FieldType::Array(Box::new(FieldType::String))
    );
    println!("  [1, \"a\", true] → Array(String) ✓");

    // Homogeneous int array
    let ints: Value = serde_json::json!([1, 2, 3]);
    assert_eq!(
        FieldType::from(&ints),
        FieldType::Array(Box::new(FieldType::Integer))
    );
    println!("  [1, 2, 3] → Array(Integer) ✓");

    // Int + Float array
    let nums: Value = serde_json::json!([1, 2.5]);
    assert_eq!(
        FieldType::from(&nums),
        FieldType::Array(Box::new(FieldType::Float))
    );
    println!("  [1, 2.5] → Array(Float) ✓");

    // Nested object
    let obj: Value = serde_json::json!({"author": "me", "count": 3});
    assert_eq!(
        FieldType::from(&obj),
        FieldType::Object(BTreeMap::from([
            ("author".into(), FieldType::String),
            ("count".into(), FieldType::Integer),
        ]))
    );
    println!("  {{author: \"me\", count: 3}} → Object(author:String, count:Integer) ✓");

    // --- Infer nested object from JSON ---
    let deep: Value = serde_json::json!({
        "meta": {"author": "me", "version": 2},
        "tags": ["rust", "arrow"]
    });
    assert_eq!(
        FieldType::from(&deep),
        FieldType::Object(BTreeMap::from([
            ("meta".into(), FieldType::Object(BTreeMap::from([
                ("author".into(), FieldType::String),
                ("version".into(), FieldType::Integer),
            ]))),
            ("tags".into(), FieldType::Array(Box::new(FieldType::String))),
        ]))
    );
    println!("  Nested JSON object → Object(meta:Object, tags:Array(String)) ✓");

    // --- Infer array of objects from JSON ---
    let arr_of_obj: Value = serde_json::json!([
        {"name": "Alice", "age": 30},
        {"name": "Bob", "age": 25}
    ]);
    assert_eq!(
        FieldType::from(&arr_of_obj),
        FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            ("age".into(), FieldType::Integer),
            ("name".into(), FieldType::String),
        ]))))
    );
    println!("  [{{name,age}}, {{name,age}}] → Array(Object(age:Int, name:String)) ✓");

    // --- Array of objects with different keys → merged ---
    let arr_mixed_obj: Value = serde_json::json!([
        {"name": "Alice"},
        {"name": "Bob", "email": "bob@x.com"}
    ]);
    assert_eq!(
        FieldType::from(&arr_mixed_obj),
        FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            ("email".into(), FieldType::String),
            ("name".into(), FieldType::String),
        ]))))
    );
    println!("  [{{name}}, {{name,email}}] → Array(Object(email:String, name:String)) ✓");

    // --- Array of objects with type conflicts in shared keys ---
    let arr_conflict: Value = serde_json::json!([
        {"val": 42},
        {"val": "hello"}
    ]);
    assert_eq!(
        FieldType::from(&arr_conflict),
        FieldType::Array(Box::new(FieldType::Object(BTreeMap::from([
            ("val".into(), FieldType::String), // Integer + String → String
        ]))))
    );
    println!("  [{{val:42}}, {{val:\"hello\"}}] → Array(Object(val:String)) ✓");

    // --- Empty array ---
    let empty_arr: Value = serde_json::json!([]);
    assert_eq!(
        FieldType::from(&empty_arr),
        FieldType::Array(Box::new(FieldType::String))
    );
    println!("  [] → Array(String) ✓");

    // --- Null value ---
    assert_eq!(FieldType::from(&Value::Null), FieldType::String);
    println!("  null → String ✓");

    // --- Simulate multi-file widening ---
    // File 1: {title: "Hello", count: 42,   meta: {a: 1}}
    // File 2: {title: "World", draft: true,  meta: {b: "x"}}
    // File 3: {count: 3.5,     tags: ["go"], meta: {a: 2.0, c: true}}
    let files: Vec<Value> = vec![
        serde_json::json!({"title": "Hello", "count": 42,   "meta": {"a": 1}}),
        serde_json::json!({"title": "World", "draft": true,  "meta": {"b": "x"}}),
        serde_json::json!({"count": 3.5,     "tags": ["go"], "meta": {"a": 2.0, "c": true}}),
    ];
    // Infer each file's fields, then widen across files per field name
    let mut field_types: BTreeMap<std::string::String, FieldType> = BTreeMap::new();
    for file in &files {
        if let Value::Object(map) = file {
            for (key, val) in map {
                let ft = FieldType::from(val);
                field_types
                    .entry(key.clone())
                    .and_modify(|existing| *existing = widen(existing.clone(), ft.clone()))
                    .or_insert(ft);
            }
        }
    }
    assert_eq!(field_types["title"], FieldType::String);
    assert_eq!(field_types["count"], FieldType::Float); // Int + Float → Float
    assert_eq!(field_types["draft"], FieldType::Boolean);
    assert_eq!(field_types["tags"], FieldType::Array(Box::new(FieldType::String)));
    assert_eq!(
        field_types["meta"],
        FieldType::Object(BTreeMap::from([
            ("a".into(), FieldType::Float),    // Int(1) + Float(2.0) → Float
            ("b".into(), FieldType::String),
            ("c".into(), FieldType::Boolean),
        ]))
    );
    println!("  Multi-file widening: title:Str, count:Float, draft:Bool, tags:Array(Str), meta:Object(a:Float,b:Str,c:Bool) ✓");

    println!("\n=== Arrow type mapping ===\n");

    let test_types = vec![
        ("Boolean", FieldType::Boolean, DataType::Boolean),
        ("Integer", FieldType::Integer, DataType::Int64),
        ("Float", FieldType::Float, DataType::Float64),
        ("String", FieldType::String, DataType::Utf8),
        (
            "Array(String)",
            FieldType::Array(Box::new(FieldType::String)),
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
        ),
        (
            "Array(Integer)",
            FieldType::Array(Box::new(FieldType::Integer)),
            DataType::List(Arc::new(Field::new("item", DataType::Int64, true))),
        ),
    ];

    for (label, ft, expected_arrow) in &test_types {
        let got = to_arrow_type(ft);
        assert_eq!(&got, expected_arrow);
        println!("  {} → {:?}  ✓", label, got);
    }

    // Object → Struct
    let obj_type = FieldType::Object(BTreeMap::from([
        ("name".into(), FieldType::String),
        ("score".into(), FieldType::Float),
    ]));
    let arrow_obj = to_arrow_type(&obj_type);
    println!("  Object(name:String, score:Float) → {:?}  ✓", arrow_obj);

    println!("\n=== Build an Arrow RecordBatch ===\n");

    // Simulate 3 markdown files with different frontmatter:
    // file1: {title: "Hello", tags: ["a","b"], draft: true}
    // file2: {title: "World", count: 42}
    // file3: {title: "Test",  tags: ["c"],    count: 3.5, draft: false}

    // After discovery + widening, the schema would be:
    // title: String, tags: Array(String), draft: Boolean, count: Float

    let schema = Schema::new(vec![
        Field::new("filename", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, true),
        Field::new(
            "tags",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            true,
        ),
        Field::new("draft", DataType::Boolean, true),
        Field::new("count", DataType::Float64, true),
    ]);

    let filename_arr = StringArray::from(vec!["blog/hello.md", "blog/world.md", "notes/test.md"]);
    let title_arr = StringArray::from(vec![Some("Hello"), Some("World"), Some("Test")]);

    // tags: [["a","b"], null, ["c"]]
    let tags_values = StringArray::from(vec!["a", "b", "c"]);
    let tags_offsets = OffsetBuffer::new(vec![0, 2, 2, 3].into());
    let tags_nulls = arrow::buffer::NullBuffer::from(vec![true, false, true]);
    let tags_arr = ListArray::new(
        Arc::new(Field::new("item", DataType::Utf8, true)),
        tags_offsets,
        Arc::new(tags_values),
        Some(tags_nulls),
    );

    let draft_arr = BooleanArray::from(vec![Some(true), None, Some(false)]);
    let count_arr = Float64Array::from(vec![None, Some(42.0), Some(3.5)]);

    let batch = RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(filename_arr),
            Arc::new(title_arr),
            Arc::new(tags_arr),
            Arc::new(draft_arr),
            Arc::new(count_arr),
        ],
    )
    .unwrap();

    println!("  Schema: {:?}", batch.schema());
    println!("  Rows:   {}", batch.num_rows());
    println!("  Cols:   {}", batch.num_columns());

    for i in 0..batch.num_rows() {
        print!("  row {}: ", i);
        for j in 0..batch.num_columns() {
            let col = batch.column(j);
            let val = arrow::util::display::array_value_to_string(col, i).unwrap();
            print!("{}={}, ", batch.schema().field(j).name(), val);
        }
        println!();
    }

    println!("\n=== Arrow RecordBatch with nested Struct ===\n");

    // Build a table from the multi-file widening result above:
    // title: Utf8, count: Float64, draft: Boolean, tags: List<Utf8>,
    // meta: Struct(a: Float64, b: Utf8, c: Boolean)

    let schema2 = Schema::new(vec![
        Field::new("filename", DataType::Utf8, false),
        Field::new("title", DataType::Utf8, true),
        Field::new("count", DataType::Float64, true),
        Field::new("draft", DataType::Boolean, true),
        Field::new(
            "tags",
            DataType::List(Arc::new(Field::new("item", DataType::Utf8, true))),
            true,
        ),
        Field::new(
            "meta",
            DataType::Struct(Fields::from(vec![
                Field::new("a", DataType::Float64, true),
                Field::new("b", DataType::Utf8, true),
                Field::new("c", DataType::Boolean, true),
            ])),
            true,
        ),
    ]);

    // 3 rows matching our 3 files
    let filename2 = StringArray::from(vec!["file1.md", "file2.md", "file3.md"]);
    let title2 = StringArray::from(vec![Some("Hello"), Some("World"), None]);
    let count2 = Float64Array::from(vec![Some(42.0), None, Some(3.5)]);
    let draft2 = BooleanArray::from(vec![None, Some(true), None]);

    // tags: [null, null, ["go"]]
    let tags2_values = StringArray::from(vec!["go"]);
    let tags2_offsets = OffsetBuffer::new(vec![0i32, 0, 0, 1].into());
    let tags2_nulls = arrow::buffer::NullBuffer::from(vec![false, false, true]);
    let tags2 = ListArray::new(
        Arc::new(Field::new("item", DataType::Utf8, true)),
        tags2_offsets,
        Arc::new(tags2_values),
        Some(tags2_nulls),
    );

    // meta struct: 3 rows
    // file1: {a: 1.0, b: null, c: null}
    // file2: {a: null, b: "x", c: null}
    // file3: {a: 2.0, b: null, c: true}
    let meta_a = Float64Array::from(vec![Some(1.0), None, Some(2.0)]);
    let meta_b = StringArray::from(vec![None, Some("x"), None]);
    let meta_c = BooleanArray::from(vec![None, None, Some(true)]);
    let meta_struct = StructArray::from(vec![
        (
            Arc::new(Field::new("a", DataType::Float64, true)),
            Arc::new(meta_a) as ArrayRef,
        ),
        (
            Arc::new(Field::new("b", DataType::Utf8, true)),
            Arc::new(meta_b) as ArrayRef,
        ),
        (
            Arc::new(Field::new("c", DataType::Boolean, true)),
            Arc::new(meta_c) as ArrayRef,
        ),
    ]);

    let batch2 = RecordBatch::try_new(
        Arc::new(schema2),
        vec![
            Arc::new(filename2),
            Arc::new(title2),
            Arc::new(count2),
            Arc::new(draft2),
            Arc::new(tags2),
            Arc::new(meta_struct),
        ],
    )
    .unwrap();

    println!("  Rows: {}, Cols: {}", batch2.num_rows(), batch2.num_columns());
    for i in 0..batch2.num_rows() {
        print!("  row {}: ", i);
        for j in 0..batch2.num_columns() {
            let col = batch2.column(j);
            let val = arrow::util::display::array_value_to_string(col, i).unwrap();
            print!("{}={}, ", batch2.schema().field(j).name(), val);
        }
        println!();
    }

    println!("\n=== All tests passed ===");
}
