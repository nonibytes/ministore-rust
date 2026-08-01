use crate::db::delete::delete_by_item_id;
use crate::db::put::{execute_put, now_ms, prepare_put};
use crate::db::sql::SQL_FIND_ITEM_ID_BY_PATH;
use crate::{MinistoreError, Result, Schema};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::Value;

/// Batch operation types.
#[derive(Debug, Clone)]
enum BatchOp {
    Put(Value),
    Delete(String),
}

/// An in-memory collection of operations.
pub struct Batch {
    ops: Vec<BatchOp>,
}

/// Applies operations immediately inside one transaction.
///
/// A writer is valid only during the callback that receives it and must not be
/// used concurrently.
pub struct BatchWriter<'writer, 'connection> {
    tx: &'writer Transaction<'connection>,
    schema: &'writer Schema,
    now_ms: i64,
    count: usize,
    failed: bool,
}

impl<'writer, 'connection> BatchWriter<'writer, 'connection> {
    pub(crate) fn new(tx: &'writer Transaction<'connection>, schema: &'writer Schema) -> Self {
        Self {
            tx,
            schema,
            now_ms: now_ms(),
            count: 0,
            failed: false,
        }
    }

    /// Insert or replace one JSON document in the current transaction.
    pub fn put_json(&mut self, doc: Value) -> Result<()> {
        self.ready()?;
        let result = prepare_put(self.schema, doc)
            .and_then(|prepared| execute_put(self.tx, self.schema, prepared, self.now_ms));
        match result {
            Ok(()) => {
                self.count += 1;
                Ok(())
            }
            Err(error) => {
                self.failed = true;
                Err(error)
            }
        }
    }

    /// Delete a path in the current transaction. A missing path is a no-op.
    pub fn delete(&mut self, path: &str) -> Result<()> {
        self.ready()?;
        if path.is_empty() {
            self.failed = true;
            return Err(MinistoreError::InvalidArgument(
                "path cannot be empty".into(),
            ));
        }

        let result = (|| {
            let item_id: Option<i64> = self
                .tx
                .query_row(SQL_FIND_ITEM_ID_BY_PATH, [path], |row| row.get(0))
                .optional()?;
            if let Some(item_id) = item_id {
                delete_by_item_id(self.tx, item_id)?;
                self.count += 1;
            }
            Ok(())
        })();
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    pub(crate) fn count(&self) -> usize {
        self.count
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    fn ready(&self) -> Result<()> {
        if self.failed {
            return Err(MinistoreError::Internal(
                "batch transaction is already marked for rollback".into(),
            ));
        }
        Ok(())
    }
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

    /// Execute all batch operations in an existing transaction.
    pub fn execute(self, tx: &Transaction<'_>, schema: &Schema) -> Result<usize> {
        let mut writer = BatchWriter::new(tx, schema);
        self.write_to(&mut writer)?;
        Ok(writer.count())
    }

    pub(crate) fn write_to(self, writer: &mut BatchWriter<'_, '_>) -> Result<()> {
        for op in self.ops {
            match op {
                BatchOp::Put(doc) => writer.put_json(doc)?,
                BatchOp::Delete(path) => writer.delete(&path)?,
            }
        }
        Ok(())
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
