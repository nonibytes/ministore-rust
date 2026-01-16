use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use regex::Regex;
use once_cell::sync::Lazy;

use crate::{MinistoreError, Result};

/// Field name validation regex: must start with letter or underscore, followed by alphanumerics or underscores.
static FIELD_NAME_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*$").unwrap()
});

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Schema {
    pub fields: BTreeMap<String, FieldSpec>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FieldSpec {
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub multi: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>, // only used for text
}

#[derive(Debug, Copy, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    Keyword,
    Text,
    Number,
    Date,
    Bool,
}

impl Schema {
    /// Create a new empty schema.
    pub fn new() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    /// Add a field to the schema.
    pub fn add_field(&mut self, name: impl Into<String>, spec: FieldSpec) {
        self.fields.insert(name.into(), spec);
    }

    /// Validate the schema for correctness.
    pub fn validate(&self) -> Result<()> {
        if self.fields.is_empty() {
            return Err(MinistoreError::Schema("schema must have at least one field".into()));
        }

        for (name, spec) in &self.fields {
            // Validate field name format
            if !FIELD_NAME_RE.is_match(name) {
                return Err(MinistoreError::Schema(format!(
                    "invalid field name '{}': must match [A-Za-z_][A-Za-z0-9_]*", name
                )));
            }

            // Reserved field names
            if matches!(name.as_str(), "path" | "created" | "updated") {
                return Err(MinistoreError::Schema(format!(
                    "field name '{}' is reserved", name
                )));
            }

            // Weight is only valid for text fields
            if let Some(weight) = spec.weight {
                if spec.field_type != FieldType::Text {
                    return Err(MinistoreError::Schema(format!(
                        "weight can only be specified for text fields, not '{}'", name
                    )));
                }
                if weight <= 0.0 {
                    return Err(MinistoreError::Schema(format!(
                        "weight for field '{}' must be positive", name
                    )));
                }
            }
        }

        Ok(())
    }

    /// Returns text fields sorted by name for deterministic FTS column order.
    /// Returns (field_name, weight) pairs.
    pub fn text_fields_in_order(&self) -> Vec<(String, f64)> {
        self.fields
            .iter()
            .filter(|(_, spec)| spec.field_type == FieldType::Text)
            .map(|(name, spec)| (name.clone(), spec.weight.unwrap_or(1.0)))
            .collect()
    }

    /// Get a field spec by name.
    pub fn get(&self, field: &str) -> Option<&FieldSpec> {
        self.fields.get(field)
    }

    /// Check if a field exists in the schema.
    pub fn has_field(&self, field: &str) -> bool {
        self.fields.contains_key(field)
    }

    /// Serialize schema to JSON string.
    pub fn to_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserialize schema from JSON string.
    pub fn from_json(json: &str) -> Result<Self> {
        Ok(serde_json::from_str(json)?)
    }
}

impl Default for Schema {
    fn default() -> Self {
        Self::new()
    }
}

impl FieldSpec {
    /// Create a text field spec.
    pub fn text(weight: Option<f64>) -> Self {
        Self {
            field_type: FieldType::Text,
            multi: false,
            weight,
        }
    }

    /// Create a keyword field spec.
    pub fn keyword(multi: bool) -> Self {
        Self {
            field_type: FieldType::Keyword,
            multi,
            weight: None,
        }
    }

    /// Create a number field spec.
    pub fn number(multi: bool) -> Self {
        Self {
            field_type: FieldType::Number,
            multi,
            weight: None,
        }
    }

    /// Create a date field spec.
    pub fn date(multi: bool) -> Self {
        Self {
            field_type: FieldType::Date,
            multi,
            weight: None,
        }
    }

    /// Create a bool field spec.
    pub fn bool() -> Self {
        Self {
            field_type: FieldType::Bool,
            multi: false,
            weight: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_validation_empty() {
        let schema = Schema::new();
        assert!(schema.validate().is_err());
    }

    #[test]
    fn test_schema_validation_valid() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        schema.add_field("priority", FieldSpec::number(false));
        assert!(schema.validate().is_ok());
    }

    #[test]
    fn test_schema_validation_invalid_name() {
        let mut schema = Schema::new();
        schema.add_field("123invalid", FieldSpec::text(None));
        assert!(schema.validate().is_err());
    }

    #[test]
    fn test_schema_validation_reserved_name() {
        let mut schema = Schema::new();
        schema.add_field("path", FieldSpec::text(None));
        assert!(schema.validate().is_err());
    }

    #[test]
    fn test_schema_validation_weight_on_non_text() {
        let mut schema = Schema::new();
        schema.add_field("priority", FieldSpec {
            field_type: FieldType::Number,
            multi: false,
            weight: Some(2.0),
        });
        assert!(schema.validate().is_err());
    }

    #[test]
    fn test_schema_validation_negative_weight() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(-1.0)));
        assert!(schema.validate().is_err());
    }

    #[test]
    fn test_text_fields_in_order() {
        let mut schema = Schema::new();
        schema.add_field("content", FieldSpec::text(Some(1.0)));
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        
        let text_fields = schema.text_fields_in_order();
        assert_eq!(text_fields.len(), 2);
        // BTreeMap is sorted by key, so content comes before title
        assert_eq!(text_fields[0], ("content".to_string(), 1.0));
        assert_eq!(text_fields[1], ("title".to_string(), 3.0));
    }

    #[test]
    fn test_schema_json_roundtrip() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("tags", FieldSpec::keyword(true));
        
        let json = schema.to_json().unwrap();
        let parsed = Schema::from_json(&json).unwrap();
        assert_eq!(schema, parsed);
    }
}
