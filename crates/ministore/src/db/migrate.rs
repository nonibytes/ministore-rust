use std::path::Path;
use rusqlite::Connection;
use crate::{Result, Schema, Index, IndexOptions};
use crate::batch::Batch;

/// Rebuild index into a new database file with a new schema.
pub fn rebuild_into_new_db(
    old_conn: &Connection, 
    new_path: &Path, 
    new_schema: Schema,
    opts: &IndexOptions
) -> Result<()> {
    // Create new index
    let new_index = Index::create(new_path, new_schema, opts.clone())?;
    
    // Iterate old items
    // Just read data_json. The new put will re-extract fields based on new schema.
    let mut stmt = old_conn.prepare("SELECT data_json FROM items")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
    
    let mut batch = Batch::new();
    for r in rows {
        let json_str = r?;
        let doc: serde_json::Value = serde_json::from_str(&json_str)?;
        batch.put_json(doc)?;
        
        if batch.len() >= 1000 {
            new_index.batch(batch)?;
            batch = Batch::new();
        }
    }
    
    if !batch.is_empty() {
        new_index.batch(batch)?;
    }
    
    Ok(())
}
