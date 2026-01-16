use rusqlite::Connection;
use crate::{Result, Schema};
use serde_json::Value;
use crate::query::{parse_query, normalize};
use crate::query::planner::compile_to_ctes;
use crate::db::put::now_ms;

/// Discover top values for a keyword field, optionally scoped by a query.
pub fn discover_values(
    conn: &Connection,
    schema: &Schema,
    field: &str,
    scoped_query: Option<&str>,
    top: usize,
) -> Result<Vec<(String, u64)>> {
    // Validate field exists in schema
    if !schema.fields.contains_key(field) {
        return Err(crate::MinistoreError::UnknownField(field.to_string()));
    }
    
    // Check if field is keyword type
    let spec = schema.fields.get(field).unwrap();
    if spec.field_type != crate::schema::FieldType::Keyword {
             return Err(crate::MinistoreError::TypeMismatch {
            field: field.to_string(),
            message: "discover_values only runs on keyword fields".into(),
        });
    }

    if let Some(query) = scoped_query {
        if !query.trim().is_empty() {
            // Scoped discovery: compile query to result CTE and aggregate against it (no LIMIT artifacts).
            let expr = parse_query(query)?;
            let normalized = normalize::normalize(expr)?;
            let compiled = compile_to_ctes(schema, normalized, now_ms())?;

            let ctes_sql: Vec<String> = compiled.ctes.iter()
                .map(|c| format!("{} AS ({})", c.name, c.sql))
                .collect();
            let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };

            // Append params: compiled params + field + top
            let mut params = compiled.params;
            params.push(field.to_string().into());
            params.push((top as i64).into());

            let sql = format!(
                "{with_clause}
                 SELECT kd.value, COUNT(DISTINCT kp.item_id) as cnt
                 FROM {result} r
                 JOIN kw_postings kp ON kp.item_id = r.item_id
                 JOIN kw_dict kd ON kd.id = kp.value_id
                 WHERE kd.field = ? AND kd.value IS NOT NULL
                 GROUP BY kd.value
                 ORDER BY cnt DESC, kd.value ASC
                 LIMIT ?",
                with_clause = with_clause,
                result = compiled.result_cte_name
            );

            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?;
            return rows.collect::<std::result::Result<_, _>>().map_err(Into::into);
        }
    }
    
    // Global discovery (fast path)
    let sql = "SELECT value, doc_freq FROM kw_dict WHERE field = ?1 ORDER BY doc_freq DESC, value ASC LIMIT ?2";
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(rusqlite::params![field, top], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
    })?;
    
    rows.collect::<std::result::Result<_, _>>().map_err(Into::into)
}

/// Get an overview of all fields with their types and document frequencies.
pub fn discover_overview(conn: &Connection, schema: &Schema) -> Result<Vec<Value>> {
    let mut results = Vec::new();
    
    for (field_name, field_spec) in &schema.fields {
        let doc_count: i64 = conn.query_row(
            "SELECT COUNT(DISTINCT item_id) FROM field_present WHERE field = ?1",
            [field_name],
            |row| row.get(0),
        ).unwrap_or(0);
        
        let mut obj = serde_json::Map::new();
        obj.insert("field".to_string(), Value::String(field_name.clone()));
        obj.insert("type".to_string(), Value::String(format!("{:?}", field_spec.field_type)));
        obj.insert("doc_count".to_string(), Value::Number(doc_count.into()));
        
        // For text fields, include weight
        if field_spec.field_type == crate::schema::FieldType::Text {
            if let Some(weight) = field_spec.weight {
                obj.insert("weight".to_string(), Value::Number(
                    serde_json::Number::from_f64(weight).unwrap_or(serde_json::Number::from(1))
                ));
            }
        }
        
        results.push(Value::Object(obj));
    }
    
    // Sort by field name
    results.sort_by(|a, b| {
        let a_name = a.get("field").and_then(|v| v.as_str()).unwrap_or("");
        let b_name = b.get("field").and_then(|v| v.as_str()).unwrap_or("");
        a_name.cmp(b_name)
    });
    
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Schema, FieldSpec};

    #[test]
    fn test_discover_overview() {
        let conn = Connection::open_in_memory().unwrap();
        
        // Set up schema
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(1.0)));
        schema.add_field("tag", FieldSpec::keyword(false));
        
        let _ = crate::db::ddl::create_base_tables(&conn); // ensure tables exist
        
        let overview = discover_overview(&conn, &schema).unwrap();
        
        assert_eq!(overview.len(), 2);
        assert!(overview[0].get("field").is_some());
    }
}
