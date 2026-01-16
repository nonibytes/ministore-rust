use rusqlite::Connection;
use crate::Result;
use super::sql::{SQL_GET_META, SQL_SET_META};

pub const META_MAGIC_KEY: &str = "ministore_magic";
pub const META_MAGIC_VAL: &str = "ministore";
pub const META_VERSION_KEY: &str = "ministore_version";
pub const META_VERSION_VAL: &str = "1";
pub const META_SCHEMA_KEY: &str = "schema_json";

/// Read a metadata value by key.
pub fn read_meta(conn: &Connection, key: &str) -> Result<Option<String>> {
    let mut stmt = conn.prepare_cached(SQL_GET_META)?;
    let mut rows = stmt.query([key])?;
    
    match rows.next()? {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// Write a metadata key-value pair (upsert).
pub fn write_meta(conn: &Connection, key: &str, val: &str) -> Result<()> {
    conn.execute(SQL_SET_META, [key, val])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ddl::create_base_tables;

    #[test]
    fn test_read_write_meta() {
        let conn = Connection::open_in_memory().unwrap();
        create_base_tables(&conn).unwrap();
        
        // Initially empty
        assert_eq!(read_meta(&conn, "test_key").unwrap(), None);
        
        // Write and read
        write_meta(&conn, "test_key", "test_value").unwrap();
        assert_eq!(read_meta(&conn, "test_key").unwrap(), Some("test_value".to_string()));
        
        // Update
        write_meta(&conn, "test_key", "updated_value").unwrap();
        assert_eq!(read_meta(&conn, "test_key").unwrap(), Some("updated_value".to_string()));
    }
}
