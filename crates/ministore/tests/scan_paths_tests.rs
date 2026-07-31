use ministore::{FieldSpec, Index, IndexOptions, MinistoreError, Result, Schema};
use serde_json::json;
use tempfile::TempDir;

fn populated_index() -> (TempDir, Index) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("scan-paths.db");
    let mut schema = Schema::new();
    schema.add_field("marker", FieldSpec::keyword(false));
    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();
    for path in ["/z", "/a_", "/aa", "/a%", "/a", "/Ω", "/é"] {
        index.put_json(json!({"path": path})).unwrap();
    }
    (temp_dir, index)
}

#[test]
fn scans_literal_prefix_in_bytewise_order() {
    let (_temp_dir, index) = populated_index();
    let mut paths = Vec::new();

    index
        .scan_paths("/a", |path| {
            paths.push(path.to_string());
            Ok(())
        })
        .unwrap();

    assert_eq!(paths, ["/a", "/a%", "/a_", "/aa"]);
}

#[test]
fn stops_when_callback_fails() {
    let (_temp_dir, index) = populated_index();
    let mut calls = 0;

    let error = index
        .scan_paths("", |_| -> Result<()> {
            calls += 1;
            Err(MinistoreError::Internal("stop".into()))
        })
        .unwrap_err();

    assert_eq!(calls, 1);
    assert!(matches!(error, MinistoreError::Internal(message) if message == "stop"));
}
