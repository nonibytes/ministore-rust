use serde_json::Value;
use crate::{Result, Schema};
use rusqlite::{Transaction, OptionalExtension};
use crate::db::put::{prepare_put, execute_put, now_ms};
use crate::db::delete::delete_by_item_id;
use crate::db::sql::SQL_FIND_ITEM_ID_BY_PATH;

/// Batch operation types.
#[derive(Debug, Clone)]
enum BatchOp {
    Put(Value),
    Delete(String),
}

/// Batch transaction for multiple operations.
pub struct Batch {
    ops: Vec<BatchOp>,
}

impl Batch {
    /// Create a new batch.
    pub fn new() -> Self {
        Self { ops: Vec::new() }
    }

    /// Add a put operation to the batch.
    pub fn put_json(&mut self, doc: Value) -> Result<()> {
        self.ops.push(BatchOp::Put(doc));
        Ok(())
    }

    /// Add a delete operation to the batch.
    pub fn delete(&mut self, path: String) -> Result<()> {
        self.ops.push(BatchOp::Delete(path));
        Ok(())
    }

    /// Execute all batch operations in a transaction.
    pub fn execute(self, tx: &Transaction, schema: &Schema) -> Result<usize> {
        let now = now_ms();
        let mut count = 0;

        for op in self.ops {
            match op {
                BatchOp::Put(doc) => {
                    let prepared = prepare_put(schema, doc)?;
                    execute_put(tx, schema, prepared, now)?;
                    count += 1;
                }
                BatchOp::Delete(path) => {
                    // Find item ID
                    let item_id: Option<i64> = tx
                        .query_row(SQL_FIND_ITEM_ID_BY_PATH, [&path], |row| row.get(0))
                        .optional()?;

                    if let Some(id) = item_id {
                        delete_by_item_id(tx, id)?;
                        count += 1;
                    }
                }
            }
        }

        Ok(count)
    }

    /// Get the number of operations in the batch.
    pub fn len(&self) -> usize {
        self.ops.len()
    }

    /// Check if the batch is empty.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

impl Default for Batch {
    fn default() -> Self {
        Self::new()
    }
}
