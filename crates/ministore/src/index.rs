use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use crate::{Result, Schema, MinistoreError, ItemView};
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct IndexOptions {
    pub cursor_ttl_ms: i64,
}

impl Default for IndexOptions {
    fn default() -> Self {
        Self {
            cursor_ttl_ms: 60 * 60 * 1000, // 1 hour
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CursorMode {
    Short,
    Full,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RankMode {
    Default,
    Recency,
    Field(String), // number/date fields only
    None,
}

#[derive(Debug, Clone)]
pub enum OutputFieldSelector {
    None,         // paths only
    All,          // entire doc
    Fields(Vec<String>), // subset
}

#[derive(Debug, Clone)]
pub struct SearchResultPage {
    pub items: Vec<Value>,          // output objects (already shaped for CLI / bindings)
    pub next_cursor: Option<String>,
    pub explain_sql: Option<String>,  // present if explain==true
    pub explain_steps: Option<Vec<String>>,
    pub has_more: bool,
}

/// Helper to shape output
fn shape_item(view: ItemView, selector: &OutputFieldSelector) -> Value {
    match selector {
        OutputFieldSelector::None => {
            let mut obj = serde_json::Map::new();
            obj.insert("path".to_string(), Value::String(view.path));
            Value::Object(obj)
        }
        OutputFieldSelector::All => {
            let mut doc = view.doc;
            if let Some(obj) = doc.as_object_mut() {
                 if !obj.contains_key("path") {
                     obj.insert("path".to_string(), Value::String(view.path));
                 }
            }
            doc
        }
        OutputFieldSelector::Fields(fields) => {
            let mut obj = serde_json::Map::new();
            obj.insert("path".to_string(), Value::String(view.path));
            
            if let Some(doc_obj) = view.doc.as_object() {
                for field in fields {
                    if let Some(val) = doc_obj.get(field) {
                        obj.insert(field.clone(), val.clone());
                    }
                }
            }
            Value::Object(obj)
        }
    }
}

/// Main index struct.
pub struct Index {
    db_path: PathBuf,
    schema: Schema,
    conn: Mutex<rusqlite::Connection>,
    #[allow(dead_code)]
    opts: IndexOptions,
}

impl Index {
    /// Create a new index at the given path with the specified schema.
    pub fn create<P: AsRef<Path>>(path: P, schema: Schema, opts: IndexOptions) -> Result<Self> {
        use crate::db::{ddl, meta, verify};
        
        let db_path = path.as_ref().to_path_buf();
        
        // Validate schema first
        schema.validate()?;
        
        // Open/create database file
        let conn = rusqlite::Connection::open(&db_path)?;
        
        // Require FTS5 only if schema uses text fields
        if !schema.text_fields_in_order().is_empty() {
            verify::require_fts5(&conn)?;
        }
        
        // Create base tables
        ddl::create_base_tables(&conn)?;
        
        // Create FTS virtual table (optional)
        if let Some(fts_ddl) = verify::build_fts_ddl(&schema)? {
            conn.execute(&fts_ddl, [])?;
        }
        
        // Set pragmas for performance
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;
        ")?;
        
        // Write metadata
        meta::write_meta(&conn, meta::META_MAGIC_KEY, meta::META_MAGIC_VAL)?;
        meta::write_meta(&conn, meta::META_VERSION_KEY, meta::META_VERSION_VAL)?;
        meta::write_meta(&conn, meta::META_SCHEMA_KEY, &schema.to_json()?)?;
        
        Ok(Self {
            db_path,
            schema,
            conn: Mutex::new(conn),
            opts,
        })
    }

    /// Open an existing index at the given path.
    pub fn open<P: AsRef<Path>>(path: P, opts: IndexOptions) -> Result<Self> {
        use crate::db::{meta, verify};
        
        let db_path = path.as_ref().to_path_buf();
        
        // Check file exists
        if !db_path.exists() {
            return Err(MinistoreError::NotFound(format!(
                "index file not found: {}",
                db_path.display()
            )));
        }
        
        // Open database
        let conn = rusqlite::Connection::open(&db_path)?;
        
        // Verify magic
        let magic = meta::read_meta(&conn, meta::META_MAGIC_KEY)?
            .ok_or_else(|| MinistoreError::Schema(
                "not a ministore database (missing magic)".into()
            ))?;
        
        if magic != meta::META_MAGIC_VAL {
            return Err(MinistoreError::Schema(format!(
                "not a ministore database (invalid magic: {})",
                magic
            )));
        }
        
        // Load schema
        let schema_json = meta::read_meta(&conn, meta::META_SCHEMA_KEY)?
            .ok_or_else(|| MinistoreError::Schema("missing schema metadata".into()))?;
        
        let schema = Schema::from_json(&schema_json)?;
        schema.validate()?;
        
        // Verify FTS only if schema has text fields
        if !schema.text_fields_in_order().is_empty() {
            verify::require_fts5(&conn)?;
            verify::verify_fts_columns(&conn, &schema)?;
        }
        
        // Set pragmas
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;
        ")?;
        
        Ok(Self {
            db_path,
            schema,
            conn: Mutex::new(conn),
            opts,
        })
    }

    /// Get the schema for this index.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Get the path to the index file.
    pub fn path(&self) -> &Path {
        &self.db_path
    }

    /// Borrow the index's persistent database connection.
    fn conn(&self) -> Result<MutexGuard<'_, rusqlite::Connection>> {
        self.conn
            .lock()
            .map_err(|_| MinistoreError::Internal("database connection lock poisoned".into()))
    }


    /// Insert or update an item from JSON.
    pub fn put_json(&self, doc: Value) -> Result<()> {
        use crate::db::put::{prepare_put, execute_put, now_ms};
        
        let prep = prepare_put(&self.schema, doc)?;
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        execute_put(&tx, &self.schema, prep, now_ms())?;
        tx.commit()?;
        Ok(())
    }

    /// Insert or update an item with explicit path and fields.
    pub fn put_fields(&self, path: &str, fields: Value) -> Result<()> {
        let mut obj = fields.as_object()
            .ok_or_else(|| MinistoreError::Schema("fields must be a JSON object".into()))?
            .clone();
        obj.insert("path".to_string(), Value::String(path.to_string()));
        self.put_json(Value::Object(obj))
    }

    /// Get an item by path.
    pub fn get(&self, path: &str) -> Result<ItemView> {
        use crate::item::ItemMeta;
        use super::db::sql::SQL_GET_ITEM_BY_PATH;
        
        let conn = self.conn()?;
        let row = conn.query_row(
            SQL_GET_ITEM_BY_PATH,
            [path],
            |row| {
                Ok((
                    row.get::<_, String>(1)?,  // data_json
                    row.get::<_, i64>(2)?,      // created_at
                    row.get::<_, i64>(3)?,      // updated_at
                ))
            },
        ).optional()?;
        
        let (data_json, created_at_ms, updated_at_ms) = row.ok_or_else(|| MinistoreError::NotFound(path.to_string()))?;
        
        let doc: Value = serde_json::from_str(&data_json)?;
        
        Ok(ItemView {
            path: path.to_string(),
            doc,
            meta: ItemMeta { created_at_ms, updated_at_ms },
        })
    }

    /// Yield item paths with the literal prefix in ascending bytewise order.
    ///
    /// The callback must not recursively operate on this index because the scan
    /// retains the index connection lock while rows are being yielded.
    pub fn scan_paths<F>(&self, prefix: &str, mut yield_path: F) -> Result<()>
    where
        F: FnMut(&str) -> Result<()>,
    {
        use crate::db::sql::SQL_SCAN_PATHS;

        let conn = self.conn()?;
        let mut statement = conn.prepare(SQL_SCAN_PATHS)?;
        let mut rows = statement.query([prefix])?;
        while let Some(row) = rows.next()? {
            let path: String = row.get(0)?;
            yield_path(&path)?;
        }
        Ok(())
    }

    /// Peek at an item (just the JSON).
    pub fn peek(&self, path: &str) -> Result<Value> {
        let view = self.get(path)?;
        Ok(view.doc)
    }

    /// Delete an item by path.
    pub fn delete(&self, path: &str) -> Result<bool> {
        use super::db::sql::SQL_FIND_ITEM_ID_BY_PATH;
        use super::db::delete::delete_by_item_id;
        
        let conn = self.conn()?;
        
        // Find item_id
        let item_id: Option<i64> = conn
            .query_row(SQL_FIND_ITEM_ID_BY_PATH, [path], |row| row.get(0))
            .optional()?;
        
        if let Some(id) = item_id {
            let tx = conn.unchecked_transaction()?;
            delete_by_item_id(&tx, id)?;
            tx.commit()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Delete items matching a query.
    pub fn delete_where(&self, query: &str) -> Result<usize> {
        // Alias to delete_query for now, which implements the logic
        self.delete_query(query)
    }

    /// Search the index with a query string.
    pub fn search(&self, query: &str, opts: crate::db::search::SearchOptions) -> Result<SearchResultPage> {
        use crate::db::search::{plan_search, run_search, search_row_to_item_view};
        use crate::cursor::{RankModeSer, CursorPosition, CursorPayload, hash_query, encode_full, decode_full, is_short_cursor_token, short_cursor_handle, make_short_handle};
        use crate::db::sql::{SQL_CLEANUP_EXPIRED_CURSORS, SQL_GET_CURSOR, SQL_PUT_CURSOR};
        use crate::db::put::now_ms;
        
        let conn = self.conn()?;

        // cleanup expired cursors opportunistically
        let now = now_ms();
        let _ = conn.execute(SQL_CLEANUP_EXPIRED_CURSORS, [now]);

        // compute expected hash for cursor validation
        let schema_json = self.schema.to_json()?;
        let rank_ser = RankModeSer::from(&opts.rank);
        let expected_hash = hash_query(&schema_json, query, &rank_ser);

        // resolve after cursor token if present
        let after_pos: Option<CursorPosition> = if let Some(tok) = &opts.after {
            if is_short_cursor_token(tok) {
                let handle = short_cursor_handle(tok).ok_or_else(|| MinistoreError::Cursor("invalid short cursor".into()))?;
                let row: Option<(String, i64)> = conn
                    .query_row(SQL_GET_CURSOR, [handle], |r| Ok((r.get(0)?, r.get(1)?)))
                    .optional()?;
                let (payload_json, expires_at) = row.ok_or_else(|| MinistoreError::Cursor("cursor expired; rerun query".into()))?;
                if expires_at < now {
                    return Err(MinistoreError::Cursor("cursor expired; rerun query".into()));
                }
                let pos: CursorPosition = serde_json::from_str(&payload_json)
                    .map_err(|e| MinistoreError::Cursor(format!("cursor json parse error: {}", e)))?;
                Some(pos)
            } else {
                Some(decode_full(tok)?)
            }
        } else {
            None
        };

        if let Some(pos) = &after_pos {
            if pos.hash != expected_hash {
                return Err(MinistoreError::Cursor("cursor does not match query/schema/rank".into()));
            }
        }

        let plan = plan_search(&conn, &self.schema, query, &opts, after_pos.as_ref())?;
        let explain_sql = if opts.explain { Some(plan.sql.clone()) } else { None };
        let explain_steps = if opts.explain { Some(plan.explain.clone()) } else { None };
        
        let (rows, has_more) = run_search(&conn, &self.schema, plan.clone(), opts.limit, opts.after.clone())?;
        
        // Generate cursor from last row if has_more
        let next_cursor = if has_more {
            if let Some(last) = rows.last() {
                // Build correct cursor payload matching ORDER BY
                let payload: CursorPayload = match &opts.rank {
                    RankMode::None => CursorPayload::None { item_id: last.item_id },
                    RankMode::Recency => CursorPayload::Recency { updated_at_ms: last.updated_at, path: last.path.clone() },
                    RankMode::Field(field) => {
                        let rv = last.score.ok_or_else(|| MinistoreError::Cursor("missing rank_value for field rank".into()))?;
                        CursorPayload::Field {
                            field: field.clone(),
                            rank_value: rv,
                            updated_at_ms: last.updated_at,
                            path: last.path.clone(),
                        }
                    }
                    RankMode::Default => {
                        if plan.requires_fts_rank {
                            let score = last.score.ok_or_else(|| MinistoreError::Cursor("missing score for fts rank".into()))?;
                            CursorPayload::Fts { score, item_id: last.item_id }
                        } else {
                            CursorPayload::Recency { updated_at_ms: last.updated_at, path: last.path.clone() }
                        }
                    }
                };

                let pos = CursorPosition { payload, hash: expected_hash.clone() };

                match opts.cursor_mode {
                    CursorMode::Full => Some(encode_full(&pos.payload, &pos.hash)?),
                    CursorMode::Short => {
                        let created_at = now;
                        let expires_at = now + self.opts.cursor_ttl_ms;
                        let payload_json = serde_json::to_string(&pos)?;
                        
                        // Retry with new handle on collision (very unlikely with 96-bit random)
                        let mut attempts = 0;
                        let handle = loop {
                            attempts += 1;
                            if attempts > 3 {
                                return Err(MinistoreError::Cursor("failed to generate unique cursor handle after 3 attempts".into()));
                            }
                            let h = make_short_handle();
                            match conn.execute(SQL_PUT_CURSOR, rusqlite::params![h, payload_json, created_at, expires_at]) {
                                Ok(_) => break h,
                                Err(rusqlite::Error::SqliteFailure(err, _)) if err.code == rusqlite::ErrorCode::ConstraintViolation => {
                                    // Handle collision - try again with new handle
                                    continue;
                                }
                                Err(e) => return Err(e.into()),
                            }
                        };
                        Some(format!("c:{}", handle))
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let items_view: Vec<ItemView> = rows.into_iter()
            .map(|r| search_row_to_item_view(r))
            .collect::<Result<Vec<_>>>()?;
            
        let items: Vec<Value> = items_view.into_iter()
            .map(|view| shape_item(view, &opts.show))
            .collect();
        
        Ok(SearchResultPage {
            items,
            next_cursor,
            explain_sql,
            explain_steps,
            has_more,
        })
    }

    /// Count items matching a query.
    pub fn count(&self, query: &str) -> Result<usize> {
        use crate::query::{parse_query, normalize};
        use crate::query::planner::compile_to_ctes;
        use crate::db::put::now_ms;
        
        let conn = self.conn()?;
        let expr = parse_query(query)?;
        let normalized = normalize::normalize(expr)?;
        let compiled = compile_to_ctes(&self.schema, normalized, now_ms())?;

        let ctes_sql: Vec<String> = compiled.ctes.iter().map(|c| format!("{} AS ({})", c.name, c.sql)).collect();
        let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };

        let sql = format!(
            "{with_clause} SELECT COUNT(*) FROM {result}",
            with_clause = with_clause,
            result = compiled.result_cte_name
        );
        let count: usize = conn.query_row(&sql, rusqlite::params_from_iter(compiled.params), |row| row.get(0))?;
        
        Ok(count)
    }

    /// Delete items matching a query.
    pub fn delete_query(&self, query: &str) -> Result<usize> {
        use crate::query::{parse_query, normalize};
        use crate::query::planner::compile_to_ctes;
        use crate::db::delete::delete_by_item_id;
        use crate::db::put::now_ms;
        
        let conn = self.conn()?;
        let expr = parse_query(query)?;
        let normalized = normalize::normalize(expr)?;
        let compiled = compile_to_ctes(&self.schema, normalized, now_ms())?;

        let ctes_sql: Vec<String> = compiled.ctes.iter().map(|c| format!("{} AS ({})", c.name, c.sql)).collect();
        let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };
        let id_sql = format!("{with_clause} SELECT item_id FROM {result}", with_clause=with_clause, result=compiled.result_cte_name);
        
        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare(&id_sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(compiled.params), |row| row.get(0))?;
        let item_ids: Vec<i64> = rows.collect::<std::result::Result<_, _>>()?;
        drop(stmt); 
        
        for id in &item_ids {
            delete_by_item_id(&tx, *id)?;
        }
        
        let count = item_ids.len();
        tx.commit()?;
        
        Ok(count)
    }

    /// Discover values for a field.
    pub fn discover_values(&self, field: &str, query: Option<&str>, top: usize) -> Result<Vec<(String, u64)>> {
        use crate::discover;
        let conn = self.conn()?;
        discover::discover_values(&conn, &self.schema, field, query, top)
    }

    /// Get an overview of all fields.
    pub fn discover_fields(&self) -> Result<Vec<Value>> {
        use crate::discover;
        let conn = self.conn()?;
        discover::discover_overview(&conn, &self.schema)
    }

    /// Compute statistics for a field.
    pub fn stats(&self, field: &str, query: Option<&str>) -> Result<Value> {
        use crate::stats;
        let conn = self.conn()?;
        stats::compute_stats(&conn, &self.schema, field, query)
    }

    /// Optimize the index (VACUUM, rebuild FTS).
    pub fn optimize(&self) -> Result<()> {
        // No-op if schema has no text fields (no FTS table exists)
        if self.schema.text_fields_in_order().is_empty() {
            return Ok(());
        }
        let conn = self.conn()?;
        // Optimize FTS
        conn.execute("INSERT INTO search(search) VALUES('optimize')", [])?;
        // VACUUM to reclaim space
        conn.execute("VACUUM", [])?;
        Ok(())
    }

    /// Migrate/rebuild the index with a new schema.
    pub fn migrate_rebuild<P: AsRef<Path>>(&self, new_path: P, new_schema: Schema) -> Result<()> {
        use crate::db::migrate::rebuild_into_new_db;
        let conn = self.conn()?;
        rebuild_into_new_db(&conn, new_path.as_ref(), new_schema, &self.opts)
    }

    /// Apply additive schema changes.
    pub fn apply_schema(&mut self, new_schema: Schema) -> Result<()> {
        // Validate new schema
        new_schema.validate()?;
        
        let current_text = self.schema.text_fields_in_order();
        let new_text = new_schema.text_fields_in_order();
        
        // Check if additive
        // 1. All existing fields must exist in new schema with same config
        for (name, spec) in &self.schema.fields {
            if let Some(new_spec) = new_schema.fields.get(name) {
                // Must match relevant properties (type, multi)
                // Strict check:
                if new_spec.field_type != spec.field_type {
                     return Err(MinistoreError::Schema(format!("field '{}' type changed", name)));
                }
                if new_spec.multi != spec.multi {
                     return Err(MinistoreError::Schema(format!("field '{}' multi changed", name)));
                }
            } else {
                return Err(MinistoreError::Schema(format!("field '{}' removed", name)));
            }
        }
        
        // 2. Identify new text fields for ALTER TABLE
        if new_text.len() < current_text.len() {
             return Err(MinistoreError::Schema("text fields removed or reordered".into()));
        }
        for (i, (name, _)) in current_text.iter().enumerate() {
            if &new_text[i].0 != name {
                 return Err(MinistoreError::Schema("text fields reordered or changed".into()));
            }
        }
        
        // Apply changes
        let conn = self.conn()?;
        
        // Add new text columns
        for i in current_text.len()..new_text.len() {
            let (name, _) = &new_text[i];
            let sql = format!("ALTER TABLE search ADD COLUMN {}", name);
            conn.execute(&sql, [])?;
        }
        
        // Update meta schema
        use crate::db::meta;
        meta::write_meta(&conn, meta::META_SCHEMA_KEY, &new_schema.to_json()?)?;
        drop(conn);

        // IMPORTANT: update in-memory schema so subsequent operations use the new schema.
        self.schema = new_schema;
        
        Ok(())
    }

    /// Stream operations into one transaction and commit only if the callback
    /// and every writer operation succeed.
    ///
    /// The callback must not recursively operate on this index because the
    /// connection lock is retained for the transaction.
    pub fn write_batch<F>(&self, write: F) -> Result<usize>
    where
        F: FnOnce(&mut crate::batch::BatchWriter<'_, '_>) -> Result<()>,
    {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let (count, failed) = {
            let mut writer = crate::batch::BatchWriter::new(&tx, &self.schema);
            write(&mut writer)?;
            (writer.count(), writer.failed())
        };
        if failed {
            return Err(MinistoreError::Internal(
                "batch operation failed; transaction rolled back".into(),
            ));
        }
        tx.commit()?;
        Ok(count)
    }

    /// Execute an in-memory batch through the streamed transaction writer.
    pub fn batch(&self, batch: crate::batch::Batch) -> Result<usize> {
        if batch.is_empty() {
            return Ok(0);
        }
        self.write_batch(|writer| batch.write_to(writer))
    }
}

#[cfg(test)]
mod connection_tests {
    use super::*;
    use crate::FieldSpec;
    use tempfile::TempDir;

    #[test]
    fn reuses_connection_between_operations() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("persistent.db");
        let mut schema = Schema::new();
        schema.add_field("active", FieldSpec::bool());
        let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();

        index
            .conn()
            .unwrap()
            .execute("CREATE TEMP TABLE connection_marker (value INTEGER)", [])
            .unwrap();
        let marker_exists: i64 = index
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_temp_master WHERE name = 'connection_marker'",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(marker_exists, 1);
    }
}
