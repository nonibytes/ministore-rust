use ministore::{Batch, FieldSpec, Index, IndexOptions, MinistoreError, Schema};
use serde_json::json;
use tempfile::TempDir;

fn new_index() -> (TempDir, Index) {
    let temp_dir = TempDir::new().unwrap();
    let mut schema = Schema::new();
    schema.add_field("tags", FieldSpec::keyword(true));
    let index = Index::create(
        temp_dir.path().join("index.db"),
        schema,
        IndexOptions::default(),
    )
    .unwrap();
    (temp_dir, index)
}

#[test]
fn streams_mixed_operations_and_maintains_document_frequency() {
    let (_temp_dir, index) = new_index();
    index
        .put_json(json!({"path": "/delete", "tags": ["old"]}))
        .unwrap();

    let count = index
        .write_batch(|writer| {
            writer.put_json(json!({"path": "/one", "tags": ["a"]}))?;
            writer.put_json(json!({"path": "/one", "tags": ["b"]}))?;
            writer.delete("/delete")?;
            writer.delete("/missing")
        })
        .unwrap();

    assert_eq!(count, 3);
    assert!(matches!(
        index.get("/delete"),
        Err(MinistoreError::NotFound(_))
    ));
    assert_eq!(index.get("/one").unwrap().doc["tags"], json!(["b"]));
    let values = index.discover_values("tags", None, 10).unwrap();
    assert_eq!(values.iter().find(|(value, _)| value == "a").unwrap().1, 0);
    assert_eq!(values.iter().find(|(value, _)| value == "b").unwrap().1, 1);
    assert_eq!(
        values.iter().find(|(value, _)| value == "old").unwrap().1,
        0
    );
}

#[test]
fn callback_failure_rolls_back_and_releases_connection() {
    let (_temp_dir, index) = new_index();
    let error = index
        .write_batch(|writer| {
            writer.put_json(json!({"path": "/rolled-back"}))?;
            Err(MinistoreError::Internal("stop".into()))
        })
        .unwrap_err();

    assert!(matches!(error, MinistoreError::Internal(message) if message == "stop"));
    assert!(matches!(
        index.get("/rolled-back"),
        Err(MinistoreError::NotFound(_))
    ));
    index.put_json(json!({"path": "/after"})).unwrap();
    assert_eq!(index.get("/after").unwrap().path, "/after");
}

#[test]
fn swallowed_operation_failure_poisons_transaction() {
    let (_temp_dir, index) = new_index();
    let error = index
        .write_batch(|writer| {
            writer.put_json(json!({"path": "/rolled-back"}))?;
            let _ = writer.put_json(json!({"missing": "path"}));
            Ok(())
        })
        .unwrap_err();

    assert!(matches!(error, MinistoreError::Internal(_)));
    assert!(matches!(
        index.get("/rolled-back"),
        Err(MinistoreError::NotFound(_))
    ));
}

#[test]
fn streams_large_input_without_an_operation_collection() {
    let (_temp_dir, index) = new_index();
    const DOCUMENTS: usize = 2_000;

    let count = index
        .write_batch(|writer| {
            for number in 0..DOCUMENTS {
                writer.put_json(json!({"path": format!("/{number:04}")}))?;
            }
            Ok(())
        })
        .unwrap();

    assert_eq!(count, DOCUMENTS);
    let mut paths = 0;
    index
        .scan_paths("", |_| {
            paths += 1;
            Ok(())
        })
        .unwrap();
    assert_eq!(paths, DOCUMENTS);
}

#[test]
fn in_memory_batch_delegates_with_the_same_counting_rules() {
    let (_temp_dir, index) = new_index();
    let mut batch = Batch::new();
    batch.put_json(json!({"path": "/one"})).unwrap();
    batch.delete("/missing".into()).unwrap();

    assert_eq!(index.batch(batch).unwrap(), 1);
    assert_eq!(index.get("/one").unwrap().path, "/one");
}
