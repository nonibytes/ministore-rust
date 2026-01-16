use serde_json::Value;
use crate::{MinistoreError, Result};

#[derive(Debug, Clone)]
pub struct ItemDoc {
    pub path: String,
    pub fields: Value, // object containing user fields (may include "path" too, but we normalize)
}

#[derive(Debug, Clone)]
pub struct ItemMeta {
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct ItemView {
    pub path: String,
    pub doc: Value,     // full document (includes "path")
    pub meta: ItemMeta, // timestamps
}

impl ItemDoc {
    /// Parse an ItemDoc from a JSON value.
    /// Enforces that "path" exists and is a string.
    pub fn from_json(v: Value) -> Result<Self> {
        let obj = v.as_object().ok_or_else(|| {
            MinistoreError::Schema("item must be a JSON object".into())
        })?;

        let path = obj.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                MinistoreError::Schema("item must have a 'path' string field".into())
            })?
            .to_string();

        if path.is_empty() {
            return Err(MinistoreError::Schema("path cannot be empty".into()));
        }

        Ok(Self {
            path,
            fields: v,
        })
    }

    /// Convert to storage JSON, ensuring "path" is included.
    pub fn to_storage_json(&self) -> Value {
        let mut obj = self.fields.as_object().cloned().unwrap_or_default();
        obj.insert("path".to_string(), Value::String(self.path.clone()));
        Value::Object(obj)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_item_doc_from_json_valid() {
        let json = json!({
            "path": "/docs/test",
            "title": "Test Document",
            "tags": ["a", "b"]
        });
        let doc = ItemDoc::from_json(json).unwrap();
        assert_eq!(doc.path, "/docs/test");
    }

    #[test]
    fn test_item_doc_from_json_missing_path() {
        let json = json!({
            "title": "Test Document"
        });
        assert!(ItemDoc::from_json(json).is_err());
    }

    #[test]
    fn test_item_doc_from_json_empty_path() {
        let json = json!({
            "path": "",
            "title": "Test Document"
        });
        assert!(ItemDoc::from_json(json).is_err());
    }

    #[test]
    fn test_item_doc_from_json_not_object() {
        let json = json!("just a string");
        assert!(ItemDoc::from_json(json).is_err());
    }

    #[test]
    fn test_item_doc_to_storage_json() {
        let json = json!({
            "path": "/docs/test",
            "title": "Test"
        });
        let doc = ItemDoc::from_json(json).unwrap();
        let storage = doc.to_storage_json();
        assert_eq!(storage["path"], "/docs/test");
        assert_eq!(storage["title"], "Test");
    }
}
