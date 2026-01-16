use serde_json::Value;
use std::path::{Path, PathBuf};
use crate::{Result, Schema, MinistoreError, ItemView};
use rusqlite::OptionalExtension;
use crate::cursor::{RankModeSer, CursorPayload};

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
        
        // Require FTS5
        verify::require_fts5(&conn)?;
        
        // Create base tables
        ddl::create_base_tables(&conn)?;
        
        // Create FTS virtual table
        let fts_ddl = verify::build_fts_ddl(&schema)?;
        conn.execute(&fts_ddl, [])?;
        
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
        
        // Close connection (Index will open its own connections on demand)
        drop(conn);
        
        Ok(Self {
            db_path,
            schema,
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
        
        // Verify FTS5 available
        verify::require_fts5(&conn)?;
        
        // Verify FTS columns match schema
        verify::verify_fts_columns(&conn, &schema)?;
        
        // Set pragmas
        conn.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA synchronous=NORMAL;
            PRAGMA foreign_keys=ON;
        ")?;
        
        // Close connection
        drop(conn);
        
        Ok(Self {
            db_path,
            schema,
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

    /// Open a connection to the database (internal helper).
    fn conn(&self) -> Result<rusqlite::Connection> {
        Ok(rusqlite::Connection::open(&self.db_path)?)
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
        )?;
        
        let (data_json, created_at, updated_at) = row;
        let doc: Value = serde_json::from_str(&data_json)?;
        
        Ok(ItemView {
            path: path.to_string(),
            doc,
            meta: ItemMeta {
                created_at_ms: created_at,
                updated_at_ms: updated_at,
            },
        })
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
        use crate::cursor::{encode_full, hash_query};
        
        let conn = self.conn()?;
        let plan = plan_search(&conn, &self.schema, query, &opts)?;
        let explain_sql = if opts.explain { Some(plan.sql.clone()) } else { None };
        let explain_steps = if opts.explain { Some(plan.explain.clone()) } else { None };
        
        let (rows, has_more) = run_search(&conn, &self.schema, plan, opts.limit, opts.after.clone())?;
        
        // Generate cursor from last row if has_more
        let next_cursor = if has_more {
            if let Some(last) = rows.last() {
                // Must reconstruct rank value based on mode
                let rank_val = match opts.rank {
                    RankMode::Default => last.score,
                    RankMode::Recency => last.score,
                    RankMode::Field(_) => last.score,
                    RankMode::None => None,
                };

                let payload = CursorPayload {
                    item_id: last.item_id,
                    rank_value: rank_val,
                    path: last.path.clone(),
                };
                
                let rank_ser = RankModeSer::from(&opts.rank);
                let schema_json = self.schema.to_json()?;
                let hash = hash_query(&schema_json, query, &rank_ser);
                
                Some(encode_full(&payload, &hash)?)
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
        })
    }

    /// Count items matching a query.
    pub fn count(&self, query: &str) -> Result<usize> {
        use crate::db::search::{plan_search, SearchOptions};
        
        let conn = self.conn()?;
        let plan = plan_search(&conn, &self.schema, query, &SearchOptions::default())?;
        
        // Replace the SELECT with COUNT(*) 
        let _count_sql = plan.sql.replace("SELECT i.id, i.path, i.data_json, i.created_at, i.updated_at, CAST(i.updated_at AS REAL) AS score", "SELECT COUNT(DISTINCT i.id)");
        
        let sql_no_limit = if let Some(idx) = plan.sql.rfind(" LIMIT ") {
            &plan.sql[..idx]
        } else {
            &plan.sql
        };
        
        let wrapped_sql = format!("SELECT COUNT(*) FROM ({})", sql_no_limit);
        
        let count: usize = conn.query_row(&wrapped_sql, rusqlite::params_from_iter(&plan.params), |row| row.get(0))?;
        
        Ok(count)
    }

    /// Delete items matching a query.
    pub fn delete_query(&self, query: &str) -> Result<usize> {
        use crate::db::search::{plan_search, SearchOptions};
        use crate::db::delete::delete_by_item_id;
        
        let conn = self.conn()?;
        let plan = plan_search(&conn, &self.schema, query, &SearchOptions { limit: 100_000, ..Default::default() })?; 
        
        let sql_no_limit = if let Some(idx) = plan.sql.rfind(" LIMIT ") {
            &plan.sql[..idx]
        } else {
            &plan.sql
        };

        // Get IDs
        let id_sql = format!("SELECT item_id FROM ({})", sql_no_limit);
        
        let tx = conn.unchecked_transaction()?;
        let mut stmt = tx.prepare(&id_sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(&plan.params), |row| row.get(0))?;
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
    pub fn apply_schema(&self, new_schema: Schema) -> Result<()> {
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
        
        Ok(())
    }

    /// Execute a batch of operations in a transaction.
    pub fn batch(&self, batch: crate::batch::Batch) -> Result<usize> {
        let mut conn = self.conn()?;
        let tx = conn.transaction()?;
        let count = batch.execute(&tx, &self.schema)?;
        tx.commit()?;
        Ok(count)
    }
}
