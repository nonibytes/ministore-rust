use rusqlite::Transaction;
use crate::Result;
use super::sql::*;

fn is_no_such_table_search(e: &rusqlite::Error) -> bool {
    match e {
        rusqlite::Error::SqliteFailure(err, Some(msg)) => {
            err.code == rusqlite::ErrorCode::Unknown && msg.contains("no such table: search")
        }
        _ => false,
    }
}

/// Delete an item by its internal ID, adjusting doc_freq counts.
pub fn delete_by_item_id(tx: &Transaction, item_id: i64) -> Result<()> {
    // Collect value_ids that need doc_freq decremented
    let value_ids: Vec<i64> = tx
        .prepare(SQL_GET_VALUE_IDS_BY_ITEM)?
        .query_map([item_id], |row| row.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    
    // Decrement doc_freq for each value_id
    for value_id in value_ids {
        tx.execute(SQL_DECREMENT_DOC_FREQ, [value_id])?;
    }
    
    // Delete from all tables (order matters for foreign keys)
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
    tx.execute(SQL_DELETE_ITEMS_BY_ID, [item_id])?;
    
    Ok(())
}
