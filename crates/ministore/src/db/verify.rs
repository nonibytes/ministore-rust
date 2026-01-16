use rusqlite::Connection;
use crate::{MinistoreError, Result, Schema};

/// Ensure SQLite has FTS5 enabled.
pub fn require_fts5(conn: &Connection) -> Result<()> {
    let has_fts5: bool = conn.query_row(
        "SELECT COUNT(*) > 0 FROM pragma_compile_options WHERE compile_options = 'ENABLE_FTS5'",
        [],
        |row| row.get(0),
    )?;
    
    if !has_fts5 {
        return Err(MinistoreError::SqliteFeatureMissing("FTS5".into()));
    }
    
    Ok(())
}

/// Build CREATE VIRTUAL TABLE search USING fts5(...) statement from schema.
pub fn build_fts_ddl(schema: &Schema) -> Result<String> {
    let text_fields = schema.text_fields_in_order();
    
    if text_fields.is_empty() {
        return Err(MinistoreError::Schema(
            "schema must have at least one text field for FTS".into()
        ));
    }
    
    let columns: Vec<String> = text_fields.iter().map(|(name, _)| name.clone()).collect();
    let columns_list = columns.join(", ");
    
    // Standard FTS5 - not using content='' due to corruption issues with delete command
    Ok(format!(
        "CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5({}, tokenize='unicode61')",
        columns_list
    ))
}

/// Verify that existing FTS table columns match schema text fields.
pub fn verify_fts_columns(conn: &Connection, schema: &Schema) -> Result<()> {
    let expected = schema.text_fields_in_order();
    let expected_names: Vec<String> = expected.iter().map(|(name, _)| name.clone()).collect();
    
    // Query FTS table columns
    let mut stmt = conn.prepare("PRAGMA table_info(search)")?;
    let all_names: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))? // column 1 is name
        .collect::<std::result::Result<Vec<_>, _>>()?;
    
    // FTS5 adds hidden columns, filter them out
    // Also ignore rank/offsets/unindexed if present (though we don't use them yet)
    // Actually, simple FTS5 columns are just the names.
    // However, if we added fields, they should be there.
    // We only care if the schema text fields are present in the table.
    // What about extra columns in table? That might be okay, but for now we enforce exact match of text fields.
    // Filter out known FTS auxiliary columns if any (unlikely in `table_info` for standard fts5, but let's be safe).
    // Actually `table_info` returns the defined columns.
    
    let actual_names: Vec<String> = all_names.into_iter()
        .filter(|n| !n.ends_with("_config") && n != "rank") 
        .collect();

    if actual_names != expected_names {
        // Detailed check
        return Err(MinistoreError::Schema(format!(
            "FTS columns mismatch; expected {:?} but found {:?}. Run 'ministore index migrate'.",
            expected_names, actual_names
        )));
    }
    
    Ok(())
}

/// Apply additive schema changes (new text fields).
pub fn apply_schema_additive(conn: &Connection, old: &Schema, new: &Schema) -> Result<()> {
    let old_fields: std::collections::HashSet<_> = old.text_fields_in_order()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    let new_fields = new.text_fields_in_order();
    
    for (name, _) in new_fields {
        if !old_fields.contains(&name) {
            // Add new FTS column
            conn.execute(&format!("ALTER TABLE search ADD COLUMN {}", name), [])?;
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::ddl::create_base_tables;
    use crate::schema::FieldSpec;

    #[test]
    fn test_require_fts5() {
        let conn = Connection::open_in_memory().unwrap();
        // With bundled feature, FTS5 should be available
        assert!(require_fts5(&conn).is_ok());
    }

    #[test]
    fn test_build_fts_ddl() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("content", FieldSpec::text(Some(1.0)));
        
        let ddl = build_fts_ddl(&schema).unwrap();
        assert!(ddl.contains("CREATE VIRTUAL TABLE"));
        assert!(ddl.contains("content, title")); // BTreeMap sorts alphabetically
        assert!(ddl.contains("fts5"));
    }

    #[test]
    fn test_build_fts_ddl_no_text_fields() {
        let mut schema = Schema::new();
        schema.add_field("tags", FieldSpec::keyword(true));
        
        assert!(build_fts_ddl(&schema).is_err());
    }

    #[test]
    fn test_verify_fts_columns() {
        let conn = Connection::open_in_memory().unwrap();
        create_base_tables(&conn).unwrap();
        
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(3.0)));
        schema.add_field("content", FieldSpec::text(Some(1.0)));
        
        let ddl = build_fts_ddl(&schema).unwrap();
        conn.execute(&ddl, []).unwrap();
        
        // Verification should succeed
        assert!(verify_fts_columns(&conn, &schema).is_ok());
    }
}
