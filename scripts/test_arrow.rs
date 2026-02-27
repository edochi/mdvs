#!/usr/bin/env rust-script
//! ```cargo
//! [dependencies]
//! arrow = "54"
//! serde_json = "1"
//! ```

use arrow::array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, StringArray, StructArray,
};
use arrow::buffer::{NullBuffer, OffsetBuffer};
use arrow::datatypes::{DataType, Field, Fields, Schema};
use arrow::record_batch::RecordBatch;
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;

// --- FieldType ---

#[derive(Debug, Clone, PartialEq)]
enum FieldType {
    Boolean,
    Integer,
    Float,
    String,
    Array(Box<FieldType>),
    Object(BTreeMap<std::string::String, FieldType>),
}

// --- FieldType → Arrow DataType ---

impl Into<DataType> for &FieldType {
    fn into(self) -> DataType {
        match self {
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

// --- Build Arrow array from JSON values ---

fn build_array(values: &[Option<&Value>], ft: &FieldType) -> ArrayRef {
    match ft {
        FieldType::Boolean => {
            let arr: BooleanArray = values.iter().map(|v| v.and_then(|v| v.as_bool())).collect();
            Arc::new(arr)
        }
        FieldType::Integer => {
            let arr: Int64Array = values.iter().map(|v| v.and_then(|v| v.as_i64())).collect();
            Arc::new(arr)
        }
        FieldType::Float => {
            let arr: Float64Array = values
                .iter()
                .map(|v| {
                    v.and_then(|v| v.as_f64().or_else(|| v.as_i64().map(|i| i as f64)))
                })
                .collect();
            Arc::new(arr)
        }
        FieldType::String => {
            let arr: StringArray = values.iter().map(|v| v.and_then(|v| v.as_str())).collect();
            Arc::new(arr)
        }
        FieldType::Array(inner) => {
            let mut offsets: Vec<i32> = vec![0];
            let mut child_values: Vec<Option<&Value>> = Vec::new();
            let mut nulls: Vec<bool> = Vec::new();

            for v in values {
                match v.and_then(|v| v.as_array()) {
                    Some(arr) => {
                        for elem in arr {
                            child_values.push(Some(elem));
                        }
                        offsets.push(child_values.len() as i32);
                        nulls.push(true);
                    }
                    None => {
                        offsets.push(*offsets.last().unwrap());
                        nulls.push(false);
                    }
                }
            }

            let child_array = build_array(&child_values, inner);
            Arc::new(ListArray::new(
                Arc::new(Field::new("item", inner.as_ref().into(), true)),
                OffsetBuffer::new(offsets.into()),
                child_array,
                Some(NullBuffer::from(nulls)),
            ))
        }
        FieldType::Object(fields) => {
            let nulls: Vec<bool> = values
                .iter()
                .map(|v| v.and_then(|v| v.as_object()).is_some())
                .collect();

            let children: Vec<(Arc<Field>, ArrayRef)> = fields
                .iter()
                .map(|(name, sub_ft)| {
                    let sub_values: Vec<Option<&Value>> = values
                        .iter()
                        .map(|v| v.and_then(|v| v.get(name.as_str())))
                        .collect();
                    (
                        Arc::new(Field::new(name, sub_ft.into(), true)),
                        build_array(&sub_values, sub_ft),
                    )
                })
                .collect();

            let (child_fields, child_arrays): (Vec<_>, Vec<_>) = children.into_iter().unzip();
            Arc::new(StructArray::new(
                child_fields.into(),
                child_arrays,
                Some(NullBuffer::from(nulls)),
            ))
        }
    }
}

// --- FileRow ---

struct FileRow {
    file_id: std::string::String,
    filename: std::string::String,
    frontmatter: Option<Value>,
    content_hash: std::string::String,
    built_at: std::string::String,
}

// --- Build files RecordBatch ---

fn build_files_batch(
    schema_fields: &[(std::string::String, FieldType)],
    files: &[FileRow],
) -> RecordBatch {
    // System columns
    let file_id_arr: StringArray = files.iter().map(|f| Some(f.file_id.as_str())).collect();
    let filename_arr: StringArray = files.iter().map(|f| Some(f.filename.as_str())).collect();
    let content_hash_arr: StringArray =
        files.iter().map(|f| Some(f.content_hash.as_str())).collect();
    let built_at_arr: StringArray = files.iter().map(|f| Some(f.built_at.as_str())).collect();

    // Build data Struct column
    let mut data_child_fields: Vec<Arc<Field>> = Vec::new();
    let mut data_child_arrays: Vec<ArrayRef> = Vec::new();

    for (name, ft) in schema_fields {
        let values: Vec<Option<&Value>> = files
            .iter()
            .map(|f| f.frontmatter.as_ref().and_then(|obj| obj.get(name.as_str())))
            .collect();
        data_child_fields.push(Arc::new(Field::new(name, ft.into(), true)));
        data_child_arrays.push(build_array(&values, ft));
    }

    // Null bitmap: data struct is null when frontmatter is None
    let data_nulls: Vec<bool> = files.iter().map(|f| f.frontmatter.is_some()).collect();
    let data_struct_type = DataType::Struct(Fields::from(
        data_child_fields
            .iter()
            .map(|f| f.as_ref().clone())
            .collect::<Vec<_>>(),
    ));
    let data_arr = StructArray::new(
        data_child_fields.into(),
        data_child_arrays,
        Some(NullBuffer::from(data_nulls)),
    );

    let schema = Schema::new(vec![
        Field::new("file_id", DataType::Utf8, false),
        Field::new("filename", DataType::Utf8, false),
        Field::new("data", data_struct_type, true),
        Field::new("content_hash", DataType::Utf8, false),
        Field::new("built_at", DataType::Utf8, false),
    ]);

    RecordBatch::try_new(
        Arc::new(schema),
        vec![
            Arc::new(file_id_arr),
            Arc::new(filename_arr),
            Arc::new(data_arr),
            Arc::new(content_hash_arr),
            Arc::new(built_at_arr),
        ],
    )
    .unwrap()
}

// ============================================================================
// Tests
// ============================================================================

fn main() {
    println!("=== Arrow files table tests ===\n");

    // --- Test 1: All scalar types with nulls ---
    {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            ("count".into(), FieldType::Integer),
            ("rating".into(), FieldType::Float),
            ("draft".into(), FieldType::Boolean),
        ];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "blog/post1.md".into(),
                frontmatter: Some(serde_json::json!({
                    "title": "Hello", "count": 42, "rating": 4.5, "draft": true
                })),
                content_hash: "hash1".into(),
                built_at: "2025-01-01T00:00:00".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "blog/post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "World", "count": 7})),
                content_hash: "hash2".into(),
                built_at: "2025-01-01T00:00:00".into(),
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "notes/idea.md".into(),
                frontmatter: None,
                content_hash: "hash3".into(),
                built_at: "2025-01-01T00:00:00".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.num_columns(), 5);

        let data = batch.column(2).as_any().downcast_ref::<StructArray>().unwrap();
        assert_eq!(data.num_columns(), 4);

        // data struct null for file 3 (no frontmatter)
        assert!(!data.is_null(0));
        assert!(!data.is_null(1));
        assert!(data.is_null(2));

        // title: "Hello", "World", null
        let titles = data
            .column_by_name("title")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(titles.value(0), "Hello");
        assert_eq!(titles.value(1), "World");
        assert!(titles.is_null(2));

        // count: 42, 7, null
        let counts = data
            .column_by_name("count")
            .unwrap()
            .as_any()
            .downcast_ref::<Int64Array>()
            .unwrap();
        assert_eq!(counts.value(0), 42);
        assert_eq!(counts.value(1), 7);
        assert!(counts.is_null(2));

        // rating: 4.5, null, null
        let ratings = data
            .column_by_name("rating")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((ratings.value(0) - 4.5).abs() < f64::EPSILON);
        assert!(ratings.is_null(1));
        assert!(ratings.is_null(2));

        // draft: true, null, null
        let drafts = data
            .column_by_name("draft")
            .unwrap()
            .as_any()
            .downcast_ref::<BooleanArray>()
            .unwrap();
        assert!(drafts.value(0));
        assert!(drafts.is_null(1));
        assert!(drafts.is_null(2));

        println!("  1. Scalar types with nulls  ✓");
    }

    // --- Test 2: Array(String) with nulls ---
    {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
        ];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "post1.md".into(),
                frontmatter: Some(
                    serde_json::json!({"title": "A", "tags": ["rust", "arrow"]}),
                ),
                content_hash: "h1".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "post2.md".into(),
                frontmatter: Some(serde_json::json!({"title": "B"})),
                content_hash: "h2".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "post3.md".into(),
                frontmatter: Some(serde_json::json!({"title": "C", "tags": ["python"]})),
                content_hash: "h3".into(),
                built_at: "2025-01-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        let data = batch.column(2).as_any().downcast_ref::<StructArray>().unwrap();
        let tags = data
            .column_by_name("tags")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();

        // File 1: ["rust", "arrow"]
        assert!(!tags.is_null(0));
        let row0 = tags.value(0);
        let row0_strs = row0.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(row0_strs.len(), 2);
        assert_eq!(row0_strs.value(0), "rust");
        assert_eq!(row0_strs.value(1), "arrow");

        // File 2: null
        assert!(tags.is_null(1));

        // File 3: ["python"]
        assert!(!tags.is_null(2));
        let row2 = tags.value(2);
        let row2_strs = row2.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(row2_strs.len(), 1);
        assert_eq!(row2_strs.value(0), "python");

        println!("  2. Array(String) with nulls  ✓");
    }

    // --- Test 3: Nested Object ---
    {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            (
                "meta".into(),
                FieldType::Object(BTreeMap::from([
                    ("author".into(), FieldType::String),
                    ("version".into(), FieldType::Float),
                ])),
            ),
        ];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "f1.md".into(),
                frontmatter: Some(serde_json::json!({
                    "title": "A", "meta": {"author": "alice", "version": 1.0}
                })),
                content_hash: "h1".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "f2.md".into(),
                frontmatter: Some(serde_json::json!({
                    "title": "B", "meta": {"author": "bob"}
                })),
                content_hash: "h2".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-3".into(),
                filename: "f3.md".into(),
                frontmatter: Some(serde_json::json!({"title": "C"})),
                content_hash: "h3".into(),
                built_at: "2025-01-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        let data = batch.column(2).as_any().downcast_ref::<StructArray>().unwrap();
        let meta = data
            .column_by_name("meta")
            .unwrap()
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();

        // File 1: meta present, both sub-fields
        assert!(!meta.is_null(0));
        let authors = meta
            .column_by_name("author")
            .unwrap()
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(authors.value(0), "alice");
        assert_eq!(authors.value(1), "bob");
        assert!(authors.is_null(2));

        let versions = meta
            .column_by_name("version")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((versions.value(0) - 1.0).abs() < f64::EPSILON);
        assert!(versions.is_null(1)); // meta exists but no version
        assert!(versions.is_null(2)); // no meta at all

        // File 2: meta present (struct not null)
        assert!(!meta.is_null(1));
        // File 3: meta absent (struct null)
        assert!(meta.is_null(2));

        println!("  3. Object(author, version) with nulls  ✓");
    }

    // --- Test 4: Array(Integer) with Integer values from JSON ---
    {
        let schema_fields = vec![(
            "scores".into(),
            FieldType::Array(Box::new(FieldType::Integer)),
        )];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "f1.md".into(),
                frontmatter: Some(serde_json::json!({"scores": [10, 20, 30]})),
                content_hash: "h1".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "f2.md".into(),
                frontmatter: Some(serde_json::json!({"scores": []})),
                content_hash: "h2".into(),
                built_at: "2025-01-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        let data = batch.column(2).as_any().downcast_ref::<StructArray>().unwrap();
        let scores = data
            .column_by_name("scores")
            .unwrap()
            .as_any()
            .downcast_ref::<ListArray>()
            .unwrap();

        // File 1: [10, 20, 30]
        let row0 = scores.value(0);
        let row0_ints = row0.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(row0_ints.len(), 3);
        assert_eq!(row0_ints.value(0), 10);
        assert_eq!(row0_ints.value(1), 20);
        assert_eq!(row0_ints.value(2), 30);

        // File 2: [] (empty array, not null)
        assert!(!scores.is_null(1));
        let row1 = scores.value(1);
        assert_eq!(row1.len(), 0);

        println!("  4. Array(Integer) and empty array  ✓");
    }

    // --- Test 5: Float field accepts integer JSON values ---
    {
        let schema_fields = vec![("val".into(), FieldType::Float)];

        let files = vec![
            FileRow {
                file_id: "id-1".into(),
                filename: "f1.md".into(),
                frontmatter: Some(serde_json::json!({"val": 42})),
                content_hash: "h1".into(),
                built_at: "2025-01-01".into(),
            },
            FileRow {
                file_id: "id-2".into(),
                filename: "f2.md".into(),
                frontmatter: Some(serde_json::json!({"val": 3.14})),
                content_hash: "h2".into(),
                built_at: "2025-01-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        let data = batch.column(2).as_any().downcast_ref::<StructArray>().unwrap();
        let vals = data
            .column_by_name("val")
            .unwrap()
            .as_any()
            .downcast_ref::<Float64Array>()
            .unwrap();
        assert!((vals.value(0) - 42.0).abs() < f64::EPSILON);
        assert!((vals.value(1) - 3.14).abs() < f64::EPSILON);

        println!("  5. Float field accepts integer JSON values  ✓");
    }

    // --- Test 6: Print table ---
    {
        let schema_fields = vec![
            ("title".into(), FieldType::String),
            (
                "tags".into(),
                FieldType::Array(Box::new(FieldType::String)),
            ),
            ("draft".into(), FieldType::Boolean),
        ];

        let files = vec![
            FileRow {
                file_id: "aaa".into(),
                filename: "blog/hello.md".into(),
                frontmatter: Some(serde_json::json!({
                    "title": "Hello", "tags": ["rust"], "draft": false
                })),
                content_hash: "h1".into(),
                built_at: "2025-06-01".into(),
            },
            FileRow {
                file_id: "bbb".into(),
                filename: "notes/idea.md".into(),
                frontmatter: Some(serde_json::json!({"title": "Idea"})),
                content_hash: "h2".into(),
                built_at: "2025-06-01".into(),
            },
            FileRow {
                file_id: "ccc".into(),
                filename: "notes/bare.md".into(),
                frontmatter: None,
                content_hash: "h3".into(),
                built_at: "2025-06-01".into(),
            },
        ];

        let batch = build_files_batch(&schema_fields, &files);
        println!("  6. Full table:");
        println!("     Schema: {}", batch.schema());
        for i in 0..batch.num_rows() {
            print!("     row {}: ", i);
            for j in 0..batch.num_columns() {
                let col = batch.column(j);
                let val = arrow::util::display::array_value_to_string(col, i).unwrap();
                print!("{}={}, ", batch.schema().field(j).name(), val);
            }
            println!();
        }
        println!("     ✓");
    }

    println!("\n=== All tests passed ===");
}
