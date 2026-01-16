use rusqlite::Connection;
use crate::{Result, Schema};
use serde_json::Value;
use crate::query::{parse_query, normalize};
use crate::query::planner::compile_to_ctes;
use crate::db::put::now_ms;

/// Compute statistics for a number or date field.
pub fn compute_stats(
    conn: &Connection,
    schema: &Schema,
    field: &str,
    scoped_query: Option<&str>,
) -> Result<Value> {
    // Handle implicit created/updated fields
    if field == "created" || field == "updated" {
        let col = if field == "created" { "created_at" } else { "updated_at" };
        
        let (count, min, max, avg): (i64, Option<f64>, Option<f64>, Option<f64>) = if let Some(query) = scoped_query {
            if !query.trim().is_empty() {
                // Scoped stats: compile to result CTE and aggregate
                let expr = parse_query(query)?;
                let normalized = normalize::normalize(expr)?;
                let compiled = compile_to_ctes(schema, normalized, now_ms())?;

                let ctes_sql: Vec<String> = compiled.ctes.iter()
                    .map(|c| format!("{} AS ({})", c.name, c.sql))
                    .collect();
                let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };

                let sql = format!(
                    "{with_clause}
                     SELECT COUNT(*), MIN(i.{col}), MAX(i.{col}), AVG(i.{col})
                     FROM {result} r
                     JOIN items i ON i.id = r.item_id",
                    with_clause = with_clause,
                    result = compiled.result_cte_name,
                    col = col
                );

                conn.query_row(
                    &sql,
                    rusqlite::params_from_iter(compiled.params),
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                )?
            } else {
                // Empty query = all items
                conn.query_row(
                    &format!("SELECT COUNT(*), MIN({col}), MAX({col}), AVG({col}) FROM items", col = col),
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                )?
            }
        } else {
            // No query = all items
            conn.query_row(
                &format!("SELECT COUNT(*), MIN({col}), MAX({col}), AVG({col}) FROM items", col = col),
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            )?
        };

        return Ok(serde_json::json!({
            "field": field,
            "count": count,
            "min": min,
            "max": max,
            "avg": avg
        }));
    }

    // Validate field exists in schema
    if !schema.fields.contains_key(field) {
        return Err(crate::MinistoreError::UnknownField(field.to_string()));
    }
    
    let field_spec = &schema.fields[field];
    
    // Determine which table to query
    let table = match field_spec.field_type {
        crate::schema::FieldType::Number => "field_number",
        crate::schema::FieldType::Date => "field_date",
        _ => {
            return Err(crate::MinistoreError::Schema(
                format!("stats not supported for field type: {:?}", field_spec.field_type)
            ));
        }
    };
    
    let (count, min, max, avg): (i64, Option<f64>, Option<f64>, Option<f64>) = if let Some(query) = scoped_query {
        if !query.trim().is_empty() {
             // Scoped stats: compile to result CTE and aggregate
             let expr = parse_query(query)?;
             let normalized = normalize::normalize(expr)?;
             let compiled = compile_to_ctes(schema, normalized, now_ms())?;

             let ctes_sql: Vec<String> = compiled.ctes.iter()
                 .map(|c| format!("{} AS ({})", c.name, c.sql))
                 .collect();
             let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };

             let mut params = compiled.params;
             params.push(field.to_string().into());

             let sql = format!(
                "{with_clause}
                 SELECT COUNT(DISTINCT fv.item_id), MIN(fv.value), MAX(fv.value), AVG(fv.value)
                 FROM {result} r
                 JOIN {table} fv ON fv.item_id = r.item_id
                 WHERE fv.field = ?",
                with_clause = with_clause,
                result = compiled.result_cte_name,
                table = table
             );

             conn.query_row(
                &sql,
                rusqlite::params_from_iter(params),
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1).ok(),
                        row.get(2).ok(),
                        row.get(3).ok(),
                    ))
                },
             )?
        } else {
             // FALLBACK: same as global
              let sql = format!(
                "SELECT COUNT(*), MIN(value), MAX(value), AVG(value) \
                 FROM {} WHERE field = ?1",
                table
            );
            conn.query_row(&sql, [field], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1).ok(),
                    row.get(2).ok(),
                    row.get(3).ok(),
                ))
            })?
        }
    } else {
         // Global stats
        let sql = format!(
            "SELECT COUNT(*), MIN(value), MAX(value), AVG(value) \
             FROM {} WHERE field = ?1",
            table
        );
        
        conn.query_row(
            &sql,
            [field],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1).ok(),
                    row.get(2).ok(),
                    row.get(3).ok(),
                ))
            },
        )?
    };
    
    let mut obj = serde_json::Map::new();
    obj.insert("field".to_string(), Value::String(field.to_string()));
    obj.insert("count".to_string(), Value::Number(count.into()));
    
    if let Some(min_val) = min {
        obj.insert("min".to_string(), Value::Number(
            serde_json::Number::from_f64(min_val).unwrap_or(serde_json::Number::from(0))
        ));
    }
    
    if let Some(max_val) = max {
        obj.insert("max".to_string(), Value::Number(
            serde_json::Number::from_f64(max_val).unwrap_or(serde_json::Number::from(0))
        ));
    }
    
    if let Some(avg_val) = avg {
        obj.insert("avg".to_string(), Value::Number(
            serde_json::Number::from_f64(avg_val).unwrap_or(serde_json::Number::from(0))
        ));
    }
    
    Ok(Value::Object(obj))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Schema, FieldSpec};
    use crate::db::ddl;

    #[test]
    fn test_compute_stats_number() {
        let conn = Connection::open_in_memory().unwrap();
        ddl::create_base_tables(&conn).unwrap();
        
        let mut schema = Schema::new();
        schema.add_field("priority", FieldSpec::number(false));
        
        // Create items first
        conn.execute(
            "INSERT INTO items (id, path, data_json, created_at, updated_at) VALUES \
             (1, '/a', '{}', 0, 0), \
             (2, '/b', '{}', 0, 0), \
             (3, '/c', '{}', 0, 0)",
            [],
        ).unwrap();
        
        // Insert test data  
        conn.execute(
            "INSERT INTO field_number (item_id, field, value) VALUES (1, 'priority', 1.0), (2, 'priority', 5.0), (3, 'priority', 3.0)",
            [],
        ).unwrap();
        
        let stats = compute_stats(&conn, &schema, "priority", None).unwrap();
        
        assert_eq!(stats.get("count").and_then(|v| v.as_i64()), Some(3));
        assert_eq!(stats.get("min").and_then(|v| v.as_f64()), Some(1.0));
        assert_eq!(stats.get("max").and_then(|v| v.as_f64()), Some(5.0));
        assert_eq!(stats.get("avg").and_then(|v| v.as_f64()), Some(3.0));
    }
}
