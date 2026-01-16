use ministore::{Index, IndexOptions, Schema, FieldSpec};
use serde_json::json;
use tempfile::TempDir;

#[test]
fn test_put_and_get() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(3.0)));
    schema.add_field("tags", FieldSpec::keyword(true));
    schema.add_field("priority", FieldSpec::number(false));
    schema.add_field("active", FieldSpec::bool());
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    // Put an item
    let doc = json!({
        "path": "/docs/test",
        "title": "Test Document",
        "tags": ["rust", "database"],
        "priority": 5,
        "active": true
    });
    
    index.put_json(doc.clone()).unwrap();
    
    // Get it back
    let retrieved = index.get("/docs/test").unwrap();
    assert_eq!(retrieved.path, "/docs/test");
    assert_eq!(retrieved.doc["title"], "Test Document");
    assert_eq!(retrieved.doc["priority"], 5);
    assert_eq!(retrieved.doc["active"], true);
}

#[test]
fn test_put_update() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    schema.add_field("priority", FieldSpec::number(false));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    // Insert
    index.put_json(json!({
        "path": "/test",
        "title": "Original",
        "priority": 1
    })).unwrap();
    
    let item1 = index.get("/test").unwrap();
    let created_at = item1.meta.created_at_ms;
    
    // Update
    std::thread::sleep(std::time::Duration::from_millis(10));
    index.put_json(json!({
        "path": "/test",
        "title": "Updated",
        "priority": 2
    })).unwrap();
    
    let item2 = index.get("/test").unwrap();
    assert_eq!(item2.doc["title"], "Updated");
    assert_eq!(item2.doc["priority"], 2);
    
    // created_at should be preserved, updated_at should change
    assert_eq!(item2.meta.created_at_ms, created_at);
    assert!(item2.meta.updated_at_ms > item2.meta.created_at_ms);
}

#[test]
fn test_delete() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    index.put_json(json!({
        "path": "/test",
        "title": "Test"
    })).unwrap();
    
    // Verify it exists
    assert!(index.get("/test").is_ok());
    
    // Delete it
    let deleted = index.delete("/test").unwrap();
    assert!(deleted);
    
    // Verify it's gone
    assert!(index.get("/test").is_err());
    
    // Deleting again should return false
    let deleted_again = index.delete("/test").unwrap();
    assert!(!deleted_again);
}

#[test]
fn test_multi_value_fields() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    schema.add_field("tags", FieldSpec::keyword(true));
    schema.add_field("scores", FieldSpec::number(true));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    index.put_json(json!({
        "path": "/test",
        "title": "Multi-value test",
        "tags": ["tag1", "tag2", "tag3"],
        "scores": [1.5, 2.7, 3.9]
    })).unwrap();
    
    let retrieved = index.get("/test").unwrap();
    assert_eq!(retrieved.doc["tags"].as_array().unwrap().len(), 3);
    assert_eq!(retrieved.doc["scores"].as_array().unwrap().len(), 3);
}

#[test]
fn test_date_field() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    schema.add_field("published", FieldSpec::date(false));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    index.put_json(json!({
        "path": "/test",
        "title": "Date test",
        "published": "2024-01-15"
    })).unwrap();
    
    let retrieved = index.get("/test").unwrap();
    assert_eq!(retrieved.doc["published"], "2024-01-15");
}

#[test]
fn test_peek() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    index.put_json(json!({
        "path": "/test",
        "title": "Test"
    })).unwrap();
    
    let doc = index.peek("/test").unwrap();
    assert_eq!(doc["title"], "Test");
    assert_eq!(doc["path"], "/test");
}

#[test]
fn test_put_fields() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    schema.add_field("priority", FieldSpec::number(false));
    
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    
    index.put_fields("/test", json!({
        "title": "Test",
        "priority": 3
    })).unwrap();
    
    let retrieved = index.get("/test").unwrap();
    assert_eq!(retrieved.path, "/test");
    assert_eq!(retrieved.doc["title"], "Test");
}
