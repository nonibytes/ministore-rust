use std::path::Path;
use rusqlite::Connection;
use crate::{Result, Schema, Index, IndexOptions};
use crate::db::put::{prepare_put, execute_put_with_timestamps};

/// Rebuild index into a new database file with a new schema.
pub fn rebuild_into_new_db(
    old_conn: &Connection, 
    new_path: &Path, 
    new_schema: Schema,
    opts: &IndexOptions
) -> Result<()> {
    // Create new index
    let new_index = Index::create(new_path, new_schema, opts.clone())?;

    // Stream old items with timestamps and reindex preserving timestamps.
    let mut stmt = old_conn.prepare("SELECT data_json, created_at, updated_at FROM items ORDER BY id")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;

    let conn = rusqlite::Connection::open(new_path)?;
    let tx = conn.unchecked_transaction()?;
    for r in rows {
        let (json_str, created_at, updated_at) = r?;
        let doc: serde_json::Value = serde_json::from_str(&json_str)?;
        let prep = prepare_put(new_index.schema(), doc)?;
        execute_put_with_timestamps(&tx, new_index.schema(), prep, created_at, updated_at)?;
    }
    tx.commit()?;

    Ok(())
}
