use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use duckdb::{Connection, params};
use mdvs_schema::{FieldInfo, FieldType};
use serde_json::Value;

use crate::embed::embedding_to_sql;
use crate::types::{ChunkData, SearchResult};

pub fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path).context("Failed to open DuckDB")?;
    Ok(conn)
}

pub fn create_tables(conn: &Connection, promoted_fields: &[&FieldInfo], dim: usize) -> Result<()> {
    // vault_meta table
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS vault_meta (
            key VARCHAR PRIMARY KEY,
            value VARCHAR
        );",
    )?;

    // mdfiles table with dynamic promoted columns
    let mut columns = vec![
        "filename VARCHAR PRIMARY KEY".to_string(),
        "content_hash VARCHAR NOT NULL".to_string(),
    ];

    for field in promoted_fields {
        let sql_type = field.field_type.sql_type();
        let col_name = sanitize_column_name(&field.name);
        columns.push(format!("{col_name} {sql_type}"));
    }

    columns.push("metadata JSON".to_string());

    let create_mdfiles = format!(
        "CREATE TABLE IF NOT EXISTS mdfiles ({});",
        columns.join(", ")
    );
    conn.execute_batch(&create_mdfiles)?;

    // chunks table
    let create_chunks = format!(
        "CREATE TABLE IF NOT EXISTS chunks (
            chunk_id VARCHAR PRIMARY KEY,
            filename VARCHAR NOT NULL REFERENCES mdfiles(filename),
            chunk_index INTEGER NOT NULL,
            heading VARCHAR,
            plain_text VARCHAR NOT NULL,
            embedding FLOAT[{dim}] NOT NULL,
            char_count INTEGER NOT NULL
        );"
    );
    conn.execute_batch(&create_chunks)?;

    Ok(())
}

pub fn store_meta(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO vault_meta (key, value) VALUES (?, ?)",
        params![key, value],
    )?;
    Ok(())
}

pub fn insert_file(
    conn: &Connection,
    filename: &str,
    promoted_fields: &[&FieldInfo],
    promoted_values: &HashMap<String, Value>,
    metadata: &Value,
    content_hash: &str,
) -> Result<()> {
    let mut col_names = vec!["filename".to_string(), "content_hash".to_string()];
    let mut placeholders = vec!["?".to_string(), "?".to_string()];
    let sql_values: Vec<String> = vec![
        format!("'{}'", escape_sql(filename)),
        format!("'{}'", escape_sql(content_hash)),
    ];

    for field in promoted_fields {
        let col_name = sanitize_column_name(&field.name);
        col_names.push(col_name);

        if let Some(val) = promoted_values.get(&field.name) {
            let sql_lit = value_to_sql_literal(val, &field.field_type);
            placeholders.push(sql_lit);
        } else {
            placeholders.push("NULL".to_string());
        }
    }

    col_names.push("metadata".to_string());
    placeholders.push(format!("'{}'", escape_sql(&metadata.to_string())));

    // Use raw SQL with literals since DuckDB params + dynamic columns is awkward
    let sql = format!(
        "INSERT INTO mdfiles ({}) VALUES ({})",
        col_names.join(", "),
        // For filename and content_hash we already have escaped values in placeholders
        // Actually let's redo this consistently with all literals
        sql_values
            .iter()
            .chain(placeholders.iter().skip(2))
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );

    conn.execute_batch(&sql)?;
    Ok(())
}

pub fn insert_chunks(
    conn: &Connection,
    chunks: &[ChunkData],
    embeddings: &[Vec<f32>],
    dim: usize,
) -> Result<()> {
    for (chunk, emb) in chunks.iter().zip(embeddings.iter()) {
        let emb_sql = embedding_to_sql(emb, dim);
        let heading_sql = chunk
            .heading
            .as_deref()
            .map(|h| format!("'{}'", escape_sql(h)))
            .unwrap_or_else(|| "NULL".to_string());

        let sql = format!(
            "INSERT INTO chunks (chunk_id, filename, chunk_index, heading, plain_text, embedding, char_count) \
             VALUES ('{}', '{}', {}, {}, '{}', {}, {})",
            escape_sql(&chunk.chunk_id),
            escape_sql(&chunk.filename),
            chunk.chunk_index,
            heading_sql,
            escape_sql(&chunk.plain_text),
            emb_sql,
            chunk.char_count,
        );
        conn.execute_batch(&sql)?;
    }
    Ok(())
}

pub fn search(
    conn: &Connection,
    query_embedding: &[f32],
    promoted_fields: &[&FieldInfo],
    dim: usize,
    limit: usize,
) -> Result<Vec<SearchResult>> {
    let query_sql = embedding_to_sql(query_embedding, dim);

    // Build the promoted column selects
    let promoted_selects: Vec<String> = promoted_fields
        .iter()
        .map(|f| format!("m.{}", sanitize_column_name(&f.name)))
        .collect();

    let extra_selects = if promoted_selects.is_empty() {
        String::new()
    } else {
        format!(", {}", promoted_selects.join(", "))
    };

    let extra_group_by = if promoted_selects.is_empty() {
        String::new()
    } else {
        format!(", {}", promoted_selects.join(", "))
    };

    let sql = format!(
        "WITH ranked_chunks AS (
            SELECT c.filename, c.heading, LEFT(c.plain_text, 120) AS snippet,
                   array_cosine_distance(c.embedding, {query_sql}) AS distance
            FROM chunks c
        )
        SELECT m.filename{extra_selects},
               MIN(rc.distance) AS distance,
               FIRST(rc.heading ORDER BY rc.distance) AS best_heading,
               FIRST(rc.snippet ORDER BY rc.distance) AS snippet
        FROM ranked_chunks rc
        JOIN mdfiles m ON rc.filename = m.filename
        GROUP BY m.filename{extra_group_by}
        ORDER BY distance
        LIMIT {limit};"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;
    let mut results = Vec::new();

    while let Some(row) = rows.next()? {
        let filename: String = row.get(0)?;

        let mut promoted = HashMap::new();
        for (i, field) in promoted_fields.iter().enumerate() {
            let col_idx = i + 1; // offset by filename
            let val = read_column_as_json(row, col_idx, &field.field_type);
            if !val.is_null() {
                promoted.insert(field.name.clone(), val);
            }
        }

        let base = promoted_fields.len() + 1;
        let distance: f64 = row.get(base)?;
        let best_heading: Option<String> = row.get(base + 1)?;
        let snippet: String = row.get(base + 2)?;

        results.push(SearchResult {
            filename,
            promoted,
            distance,
            best_heading,
            snippet,
        });
    }

    Ok(results)
}

fn read_column_as_json(row: &duckdb::Row<'_>, idx: usize, field_type: &FieldType) -> Value {
    match field_type {
        FieldType::String | FieldType::Date | FieldType::Enum => row
            .get::<_, Option<String>>(idx)
            .ok()
            .flatten()
            .map(Value::String)
            .unwrap_or(Value::Null),
        FieldType::Boolean => row
            .get::<_, Option<bool>>(idx)
            .ok()
            .flatten()
            .map(Value::Bool)
            .unwrap_or(Value::Null),
        FieldType::Integer => row
            .get::<_, Option<i64>>(idx)
            .ok()
            .flatten()
            .map(|i| Value::Number(i.into()))
            .unwrap_or(Value::Null),
        FieldType::Float => row
            .get::<_, Option<f64>>(idx)
            .ok()
            .flatten()
            .and_then(|f| serde_json::Number::from_f64(f).map(Value::Number))
            .unwrap_or(Value::Null),
        FieldType::StringArray => {
            // DuckDB VARCHAR[] comes back as a string representation
            // Read as string and keep as-is for display
            row.get::<_, Option<String>>(idx)
                .ok()
                .flatten()
                .map(Value::String)
                .unwrap_or(Value::Null)
        }
    }
}

fn value_to_sql_literal(val: &Value, field_type: &FieldType) -> String {
    match field_type {
        FieldType::String | FieldType::Enum => match val {
            Value::String(s) => format!("'{}'", escape_sql(s)),
            other => format!("'{}'", escape_sql(&other.to_string())),
        },
        FieldType::Date => match val {
            Value::String(s) => {
                // Take just the date part (YYYY-MM-DD) for DATE columns
                let date_str = if s.len() >= 10 { &s[..10] } else { s };
                format!("'{date_str}'::DATE")
            }
            _ => "NULL".to_string(),
        },
        FieldType::Boolean => match val {
            Value::Bool(b) => b.to_string(),
            _ => "NULL".to_string(),
        },
        FieldType::Integer => match val {
            Value::Number(n) => n.to_string(),
            _ => "NULL".to_string(),
        },
        FieldType::Float => match val {
            Value::Number(n) => n.to_string(),
            _ => "NULL".to_string(),
        },
        FieldType::StringArray => {
            match val {
                Value::Array(arr) => {
                    let items: Vec<String> = arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => format!("'{}'", escape_sql(s)),
                            other => format!("'{}'", escape_sql(&other.to_string())),
                        })
                        .collect();
                    format!("[{}]", items.join(", "))
                }
                // Scalar wrapped by coerce_value should already be an array,
                // but handle just in case
                Value::String(s) => format!("['{}']", escape_sql(s)),
                _ => "NULL".to_string(),
            }
        }
    }
}

fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

fn sanitize_column_name(name: &str) -> String {
    // Quote the column name to handle reserved words and special chars
    format!("\"{}\"", name.replace('"', "\"\""))
}
