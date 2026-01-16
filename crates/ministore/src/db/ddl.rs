// DDL creation for all base tables; FTS DDL is generated in code because it depends on schema text fields.

pub const DDL_BASE: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key TEXT PRIMARY KEY,
  value TEXT
);

CREATE TABLE IF NOT EXISTS items (
  id INTEGER PRIMARY KEY,
  path TEXT UNIQUE NOT NULL,
  data_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_items_path ON items(path);
CREATE INDEX IF NOT EXISTS idx_items_updated ON items(updated_at);
CREATE INDEX IF NOT EXISTS idx_items_created ON items(created_at);

CREATE TABLE IF NOT EXISTS field_present (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  PRIMARY KEY (item_id, field)
);
CREATE INDEX IF NOT EXISTS idx_present_field ON field_present(field, item_id);

CREATE TABLE IF NOT EXISTS kw_dict (
  id INTEGER PRIMARY KEY,
  field TEXT NOT NULL,
  value TEXT NOT NULL,
  doc_freq INTEGER DEFAULT 0,
  UNIQUE (field, value)
);
CREATE INDEX IF NOT EXISTS idx_kw_dict_lookup ON kw_dict(field, value);

CREATE TABLE IF NOT EXISTS kw_postings (
  field TEXT NOT NULL,
  value_id INTEGER NOT NULL REFERENCES kw_dict(id),
  item_id INTEGER NOT NULL REFERENCES items(id),
  PRIMARY KEY (value_id, item_id)
);
CREATE INDEX IF NOT EXISTS idx_kw_postings_item ON kw_postings(item_id);
CREATE INDEX IF NOT EXISTS idx_kw_postings_field ON kw_postings(field, value_id);

CREATE TABLE IF NOT EXISTS field_number (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value REAL NOT NULL,
  PRIMARY KEY (item_id, field, value)
);
CREATE INDEX IF NOT EXISTS idx_num_lookup ON field_number(field, value);

CREATE TABLE IF NOT EXISTS field_date (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY (item_id, field, value)
);
CREATE INDEX IF NOT EXISTS idx_date_lookup ON field_date(field, value);

CREATE TABLE IF NOT EXISTS field_bool (
  item_id INTEGER NOT NULL REFERENCES items(id),
  field TEXT NOT NULL,
  value INTEGER NOT NULL,
  PRIMARY KEY (item_id, field)
);
CREATE INDEX IF NOT EXISTS idx_bool_lookup ON field_bool(field, value);

CREATE TABLE IF NOT EXISTS cursor_store (
  handle TEXT PRIMARY KEY,
  payload TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_cursor_expires ON cursor_store(expires_at);
"#;

use rusqlite::Connection;
use crate::Result;

/// Execute all base DDL statements on a connection.
pub fn create_base_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(DDL_BASE)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_base_tables() {
        let conn = Connection::open_in_memory().unwrap();
        create_base_tables(&conn).unwrap();
        
        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        
        assert!(tables.contains(&"meta".to_string()));
        assert!(tables.contains(&"items".to_string()));
        assert!(tables.contains(&"field_present".to_string()));
        assert!(tables.contains(&"kw_dict".to_string()));
        assert!(tables.contains(&"kw_postings".to_string()));
        assert!(tables.contains(&"field_number".to_string()));
        assert!(tables.contains(&"field_date".to_string()));
        assert!(tables.contains(&"field_bool".to_string()));
        assert!(tables.contains(&"cursor_store".to_string()));
    }
}
