use ministore::{Index, IndexOptions, Schema, FieldSpec};
use tempfile::TempDir;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_create_and_open() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Create schema
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("content", FieldSpec::text(Some(1.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        schema.add_field("priority", FieldSpec::number(false));
        
        // Create index
        let index = Index::create(&db_path, schema.clone(), IndexOptions::default()).unwrap();
        assert_eq!(index.schema(), &schema);
        assert_eq!(index.path(), db_path);
        
        // Open index
        let opened = Index::open(&db_path, IndexOptions::default()).unwrap();
        assert_eq!(opened.schema(), &schema);
        assert_eq!(opened.path(), db_path);
    }

    #[test]
    fn test_index_create_validates_schema() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Empty schema should fail
        let schema = Schema::new();
        assert!(Index::create(&db_path, schema, IndexOptions::default()).is_err());
    }

    #[test]
    fn test_index_open_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("nonexistent.db");
        
        assert!(Index::open(&db_path, IndexOptions::default()).is_err());
    }

    #[test]
    fn test_index_open_invalid_magic() {
        use rusqlite::Connection;
        
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Create a regular SQLite database without ministore magic
        let conn = Connection::open(&db_path).unwrap();
        conn.execute("CREATE TABLE test (id INTEGER)", []).unwrap();
        drop(conn);
        
        // Opening should fail due to missing magic
        assert!(Index::open(&db_path, IndexOptions::default()).is_err());
    }

    #[test]
    fn test_index_schema_persists() {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        
        // Create with specific schema
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(5.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        schema.add_field("active", FieldSpec::bool());
        
        Index::create(&db_path, schema.clone(), IndexOptions::default()).unwrap();
        
        // Open and verify schema matches
        let opened = Index::open(&db_path, IndexOptions::default()).unwrap();
        let loaded_schema = opened.schema();
        
        assert_eq!(loaded_schema.fields.len(), schema.fields.len());
        assert_eq!(
            loaded_schema.get("title").unwrap().weight,
            Some(5.0)
        );
        assert!(loaded_schema.get("tags").unwrap().multi);
    }
}
