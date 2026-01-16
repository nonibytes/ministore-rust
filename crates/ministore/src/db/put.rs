use rusqlite::Transaction;
use serde_json::Value;
use crate::{Result, Schema, FieldType, FieldSpec, MinistoreError};
use chrono::Utc;
use std::collections::HashSet;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct PutPrepared {
    pub path: String,
    pub data_json: String,
    pub text_cols: Vec<Option<String>>, // ordered to match FTS columns
    pub keyword_fields: Vec<(String, Vec<String>)>,
    pub number_fields: Vec<(String, Vec<f64>)>,
    pub date_fields: Vec<(String, Vec<i64>)>,
    pub bool_fields: Vec<(String, bool)>,
    pub present_fields: Vec<String>,
}

/// Get current time in epoch milliseconds.
pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Pre-validate and coerce doc according to schema.
pub fn prepare_put(schema: &Schema, doc: Value) -> Result<PutPrepared> {
    let obj = doc.as_object()
        .ok_or_else(|| MinistoreError::Schema("document must be a JSON object".into()))?;
    
    // Extract path
    let path = obj.get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| MinistoreError::Schema("document must have 'path' string field".into()))?
        .to_string();
    
    if path.is_empty() {
        return Err(MinistoreError::Schema("path cannot be empty".into()));
    }
    
    let mut text_cols = Vec::new();
    let mut keyword_fields = Vec::new();
    let mut number_fields = Vec::new();
    let mut date_fields = Vec::new();
    let mut bool_fields = Vec::new();
    let mut present_fields = Vec::new();
    
    // Build FTS columns in schema order
    let text_field_names = schema.text_fields_in_order();
    for (field_name, _) in &text_field_names {
        if let Some(value) = obj.get(field_name) {
            if let Some(s) = value.as_str() {
                text_cols.push(Some(s.to_string()));
                present_fields.push(field_name.clone());
            } else {
                return Err(MinistoreError::TypeMismatch {
                    field: field_name.clone(),
                    message: "expected string for text field".into(),
                });
            }
        } else {
            text_cols.push(None);
        }
    }
    
    // Process all schema fields
    for (field_name, spec) in &schema.fields {
        if let Some(value) = obj.get(field_name) {
            match spec.field_type {
                FieldType::Keyword => {
                    let values = extract_keyword_values(field_name, value, spec)?;
                    if !values.is_empty() {
                        keyword_fields.push((field_name.clone(), values));
                        if !present_fields.contains(field_name) {
                            present_fields.push(field_name.clone());
                        }
                    }
                }
                FieldType::Text => {
                    // Already handled in FTS columns above
                }
                FieldType::Number => {
                    let values = extract_number_values(field_name, value, spec)?;
                    if !values.is_empty() {
                        number_fields.push((field_name.clone(), values));
                        if !present_fields.contains(field_name) {
                            present_fields.push(field_name.clone());
                        }
                    }
                }
                FieldType::Date => {
                    let values = extract_date_values(field_name, value, spec)?;
                    if !values.is_empty() {
                        date_fields.push((field_name.clone(), values));
                        if !present_fields.contains(field_name) {
                            present_fields.push(field_name.clone());
                        }
                    }
                }
                FieldType::Bool => {
                    let b_val = if let Some(b) = value.as_bool() {
                        Some(b)
                    } else if let Some(s) = value.as_str() {
                        if s == "true" { Some(true) } else if s == "false" { Some(false) } else { None }
                    } else {
                        None
                    };

                    if let Some(b) = b_val {
                        bool_fields.push((field_name.clone(), b));
                        if !present_fields.contains(field_name) {
                            present_fields.push(field_name.clone());
                        }
                    } else {
                        return Err(MinistoreError::TypeMismatch {
                            field: field_name.clone(),
                            message: "expected boolean".into(),
                        });
                    }
                }
            }
        }
    }
    
    // Store full document as JSON
    let data_json = serde_json::to_string(&doc)?;
    
    Ok(PutPrepared {
        path,
        data_json,
        text_cols,
        keyword_fields,
        number_fields,
        date_fields,
        bool_fields,
        present_fields,
    })
}

fn extract_keyword_values(field_name: &str, value: &Value, spec: &FieldSpec) -> Result<Vec<String>> {
    if let Some(s) = value.as_str() {
        Ok(vec![s.to_string()])
    } else if let Some(arr) = value.as_array() {
        if !spec.multi && arr.len() > 1 {
            return Err(MinistoreError::TypeMismatch {
                field: field_name.into(),
                message: "field is not multi-valued but received array with multiple values".into(),
            });
        }
        arr.iter()
            .map(|v| v.as_str()
                .ok_or_else(|| MinistoreError::TypeMismatch {
                    field: field_name.into(),
                    message: "expected string in array".into(),
                })
                .map(|s| s.to_string()))
            .collect()
    } else if let Some(n) = value.as_f64() {
        // Coerce number to string for keywords? Maybe not specific requirement but helpful.
        Ok(vec![n.to_string()])
    } else if let Some(b) = value.as_bool() {
        Ok(vec![b.to_string()])
    } else {
        Err(MinistoreError::TypeMismatch {
            field: field_name.into(),
            message: "expected string or array of strings".into(),
        })
    }
}

fn extract_number_values(field_name: &str, value: &Value, spec: &FieldSpec) -> Result<Vec<f64>> {
    if let Some(n) = value.as_f64() {
        Ok(vec![n])
    } else if let Some(n) = value.as_i64() {
        Ok(vec![n as f64])
    } else if let Some(s) = value.as_str() {
        // Coerce string to number
        s.parse::<f64>().map(|n| vec![n]).map_err(|_| MinistoreError::TypeMismatch {
            field: field_name.into(),
            message: format!("invalid number format in string: {}", s),
        })
    } else if let Some(arr) = value.as_array() {
        if !spec.multi && arr.len() > 1 {
            return Err(MinistoreError::TypeMismatch {
                field: field_name.into(),
                message: "field is not multi-valued but received array with multiple values".into(),
            });
        }
        arr.iter()
            .map(|v| v.as_f64()
                .or_else(|| v.as_i64().map(|i| i as f64))
                .or_else(|| v.as_str().and_then(|s| s.parse::<f64>().ok()))
                .ok_or_else(|| MinistoreError::TypeMismatch {
                    field: field_name.into(),
                    message: "expected number in array".into(),
                }))
            .collect()
    } else {
        Err(MinistoreError::TypeMismatch {
            field: field_name.into(),
            message: "expected number or array of numbers".into(),
        })
    }
}

fn extract_date_values(field_name: &str, value: &Value, spec: &FieldSpec) -> Result<Vec<i64>> {
    if let Some(s) = value.as_str() {
        Ok(vec![parse_date_to_epoch_ms(s)?])
    } else if let Some(arr) = value.as_array() {
        if !spec.multi && arr.len() > 1 {
            return Err(MinistoreError::TypeMismatch {
                field: field_name.into(),
                message: "field is not multi-valued but received array with multiple values".into(),
            });
        }
        arr.iter()
            .map(|v| {
                let s = v.as_str()
                    .ok_or_else(|| MinistoreError::TypeMismatch {
                        field: field_name.into(),
                        message: "expected ISO date string in array".into(),
                    })?;
                parse_date_to_epoch_ms(s)
            })
            .collect()
    } else {
        Err(MinistoreError::TypeMismatch {
            field: field_name.into(),
            message: "expected ISO date string or array of date strings".into(),
        })
    }
}

fn parse_date_to_epoch_ms(s: &str) -> Result<i64> {
    use chrono::NaiveDate;
    
    // Try parsing as YYYY-MM-DD
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
    }
    
    // Try parsing as ISO 8601 datetime
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    
    Err(MinistoreError::TypeMismatch {
        field: "date".into(),
        message: format!("invalid date format: {}", s),
    })
}

/// Execute prepared put operation, preserving provided timestamps.
/// Used for migrations/rebuilds.
pub fn execute_put_with_timestamps(
    tx: &Transaction,
    schema: &Schema,
    prep: PutPrepared,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> Result<()> {
    // Upsert items row with timestamps, then same indexing steps as execute_put.
    let (item_id, _created) = upsert_item_row_with_timestamps(tx, &prep.path, &prep.data_json, created_at_ms, updated_at_ms)?;
    execute_put_internal(tx, schema, prep, item_id, created_at_ms, updated_at_ms)
}

/// Execute prepared put operation.
pub fn execute_put(tx: &Transaction, schema: &Schema, prep: PutPrepared, now_ms: i64) -> Result<()> {
    let (item_id, created_at_ms) = upsert_item_row(tx, &prep.path, &prep.data_json, now_ms)?;
    execute_put_internal(tx, schema, prep, item_id, created_at_ms, now_ms)
}

fn execute_put_internal(
    tx: &Transaction,
    schema: &Schema,
    prep: PutPrepared,
    item_id: i64,
    _created_at_ms: i64,
    _updated_at_ms: i64,
) -> Result<()> {
    use super::sql::*;
    fn is_no_such_table_search(e: &rusqlite::Error) -> bool {
        match e {
            rusqlite::Error::SqliteFailure(err, Some(msg)) => {
                err.code == rusqlite::ErrorCode::Unknown && msg.contains("no such table: search")
            }
            _ => false,
        }
    }

    // Track old keyword value_ids BEFORE deleting postings.
    let old_value_ids: HashSet<i64> = tx
        .prepare(SQL_GET_VALUE_IDS_BY_ITEM)?
        .query_map([item_id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;

    // Delete old entries
    tx.execute(SQL_DELETE_POSTINGS_BY_ITEM, [item_id])?;
    tx.execute(SQL_DELETE_NUMBER_BY_ITEM, [item_id])?;
    tx.execute(SQL_DELETE_DATE_BY_ITEM, [item_id])?;
    tx.execute(SQL_DELETE_BOOL_BY_ITEM, [item_id])?;
    tx.execute(SQL_DELETE_PRESENT_BY_ITEM, [item_id])?;
    if let Err(e) = tx.execute(SQL_DELETE_SEARCH_ROW, [item_id]) {
        if !is_no_such_table_search(&e) {
            return Err(e.into());
        }
    }

    // Insert field_present
    for field in &prep.present_fields {
        tx.execute(SQL_INSERT_FIELD_PRESENT, rusqlite::params![item_id, field])?;
    }

    // Insert keywords with dictionary+postings
    let mut new_value_ids = HashSet::new();
    for (field, values) in &prep.keyword_fields {
        for value in values {
            tx.execute(SQL_INSERT_OR_IGNORE_KW_DICT, rusqlite::params![field, value])?;
            let value_id: i64 = tx.query_row(
                SQL_GET_KW_DICT_ID,
                rusqlite::params![field, value],
                |row| row.get(0),
            )?;

            // Insert posting (we deleted old postings already, so this will usually insert)
            tx.execute(
                SQL_INSERT_OR_IGNORE_KW_POSTING,
                rusqlite::params![field, value_id, item_id],
            )?;

            // Correct doc_freq semantics:
            // increment only if this value was NOT previously associated with this item.
            if !old_value_ids.contains(&value_id) {
                tx.execute(SQL_INCREMENT_DOC_FREQ, [value_id])?;
            }

            new_value_ids.insert(value_id);
        }
    }

    // Decrement doc_freq for values no longer present
    for value_id in old_value_ids.difference(&new_value_ids) {
        tx.execute(SQL_DECREMENT_DOC_FREQ, [*value_id])?;
    }

    // Insert numbers
    for (field, values) in &prep.number_fields {
        for value in values {
            tx.execute(SQL_INSERT_FIELD_NUMBER, rusqlite::params![item_id, field, value])?;
        }
    }

    // Insert dates
    for (field, values) in &prep.date_fields {
        for value in values {
            tx.execute(SQL_INSERT_FIELD_DATE, rusqlite::params![item_id, field, value])?;
        }
    }

    // Insert bools
    for (field, value) in &prep.bool_fields {
        let int_value = if *value { 1 } else { 0 };
        tx.execute(SQL_INSERT_FIELD_BOOL, rusqlite::params![item_id, field, int_value])?;
    }

    // Insert FTS row (if any text fields)
    let text_fields = schema.text_fields_in_order();
    if !text_fields.is_empty() {
        let placeholders: Vec<_> = (0..=text_fields.len()).map(|_| "?").collect();
        let sql = format!(
            "INSERT INTO search(rowid, {}) VALUES({})",
            text_fields.iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>().join(", "),
            placeholders.join(", ")
        );

        let mut params: Vec<rusqlite::types::Value> = vec![item_id.into()];
        for text_val in &prep.text_cols {
            params.push(text_val.clone().into());
        }
        if let Err(e) = tx.execute(&sql, rusqlite::params_from_iter(params)) {
            if !is_no_such_table_search(&e) {
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// Internal helper: insert/update items table and return (item_id, created_at_ms).
pub fn upsert_item_row(tx: &Transaction, path: &str, data_json: &str, now_ms: i64) -> Result<(i64, i64)> {
    use super::sql::*;
    
    // Check if item exists
    let existing: Option<(i64, i64)> = tx
        .query_row(
            SQL_GET_ITEM_BY_PATH,
            [path],
            |row| Ok((row.get(0)?, row.get(2)?)), // id, created_at
        )
        .optional()?;
    
    if let Some((id, created_at)) = existing {
        // Update existing
        tx.execute(
            "UPDATE items SET data_json = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![data_json, now_ms, id],
        )?;
        Ok((id, created_at))
    } else {
        // Insert new
        tx.execute(
            "INSERT INTO items(path, data_json, created_at, updated_at) VALUES(?1, ?2, ?3, ?4)",
            rusqlite::params![path, data_json, now_ms, now_ms],
        )?;
        let id = tx.last_insert_rowid();
        Ok((id, now_ms))
    }
}

/// Upsert with explicit timestamps (used for rebuild).
pub fn upsert_item_row_with_timestamps(
    tx: &Transaction,
    path: &str,
    data_json: &str,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> Result<(i64, i64)> {
    use super::sql::*;

    let existing: Option<(i64, i64)> = tx
        .query_row(
            SQL_GET_ITEM_BY_PATH,
            [path],
            |row| Ok((row.get(0)?, row.get(2)?)), // id, created_at
        )
        .optional()?;

    if let Some((id, created_at)) = existing {
        tx.execute(
            "UPDATE items SET data_json = ?1, created_at = ?2, updated_at = ?3 WHERE id = ?4",
            rusqlite::params![data_json, created_at_ms, updated_at_ms, id],
        )?;
        Ok((id, created_at))
    } else {
        tx.execute(
            "INSERT INTO items(path, data_json, created_at, updated_at) VALUES(?1, ?2, ?3, ?4)",
            rusqlite::params![path, data_json, created_at_ms, updated_at_ms],
        )?;
        let id = tx.last_insert_rowid();
        Ok((id, created_at_ms))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_prepare_put_basic() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(1.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        schema.add_field("priority", FieldSpec::number(false));
        
        let doc = json!({
            "path": "/test/doc",
            "title": "Test Document",
            "tags": ["a", "b"],
            "priority": 5
        });
        
        let prep = prepare_put(&schema, doc).unwrap();
        assert_eq!(prep.path, "/test/doc");
        assert_eq!(prep.text_cols.len(), 1);
        assert_eq!(prep.text_cols[0], Some("Test Document".to_string()));
        assert_eq!(prep.keyword_fields.len(), 1);
        assert_eq!(prep.number_fields.len(), 1);
    }

    #[test]
    fn test_parse_date() {
        let epoch = parse_date_to_epoch_ms("2024-01-15").unwrap();
        assert!(epoch > 0);
        
        let epoch2 = parse_date_to_epoch_ms("2024-01-15T10:30:00Z").unwrap();
        assert!(epoch2 > epoch);
    }
}
