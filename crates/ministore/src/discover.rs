use rusqlite::Connection;
use crate::{Result, Schema};
use serde_json::Value;
use crate::db::search::{plan_search, SearchOptions};

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
            // Scoped discovery: plan the query to get the item_ids, then join with postings
            let plan = plan_search(conn, schema, query, &SearchOptions { limit: 100_000, ..Default::default() }, None)?; // large limit for aggregation
            
            // Extract the inner SQL (CTE + main select without limit/order) or just use the whole thing as a subquery.
            // Search query returns: item_id, path, data, created, updated, score.
            // We only need item_id.
            
            // We can wrap the search query:
            // WITH ... SELECT i.id, ... FROM ...
            // ->
            // WITH ... SELECT kp.value_id, COUNT(*) as cnt 
            // FROM ({search_sql}) s
            // JOIN kw_postings kp ON kp.item_id = s.item_id
            // JOIN kw_dict kd ON kd.id = kp.value_id
            // WHERE kd.field = ?field
            // GROUP BY kp.value_id
            // ORDER BY cnt DESC
            // LIMIT ?top
            
            // However, the `plan.sql` usually has a LIMIT at the end. We might want to remove it or set it very high.
            // We set it to 100_000 above.
            
            // We need to inject the field param.
            let mut params = plan.params.clone();
            params.push(field.to_string().into()); // ?N+1
            params.push((top as i64).into()); // ?N+2
            
            let field_idx = params.len() - 1; // 1-based index in SQL will be len-1 (0-based) + 1 = len
            let top_idx = params.len(); 
            
            // Modify search SQL to select only item_id
            // The plan.sql selects "i.id, i.path, ..."
            // We can alias it as `s` and select `s.item_id` (since column name is `id` or `item_id` in CTE, but main select has `i.id`).
            // Actually, `run_search` expects cols 0..5.
            // Let's rely on column 0 being item_id.
            
            let sql = format!(
                "WITH subset AS ({search_sql}) \
                 SELECT kd.value, COUNT(*) as cnt \
                 FROM subset s \
                 JOIN kw_postings kp ON kp.item_id = s.item_id \
                 JOIN kw_dict kd ON kd.id = kp.value_id \
                 WHERE kd.field = ?{field_idx} \
                 GROUP BY kd.value \
                 ORDER BY cnt DESC, kd.value ASC \
                 LIMIT ?{top_idx}",
                search_sql = plan.sql,
                field_idx = field_idx,
                top_idx = top_idx
            );
            
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as u64))
            })?;
            
            return rows.collect::<std::result::Result<_, _>>().map_err(Into::into);
        }
    }
    
    // Global discovery (fast path)
    // Uses doc_freq checks or direct aggregation on postings if doc_freq is not tracked per-value (it is tracked in kw_dict doc_freq col)
    // Actually kw_dict has `doc_freq` column which is maintained by put/delete.
    // So we can just query kw_dict!
    
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
