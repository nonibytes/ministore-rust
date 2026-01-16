use rusqlite::Connection;
use crate::{Result, Schema, ItemView, item::ItemMeta};
use crate::query::{parse_query, normalize};
use crate::query::planner::{compile_to_ctes, build_search_sql};
use crate::index::{RankMode, OutputFieldSelector, CursorMode};
use crate::cursor::{RankModeSer, hash_query, decode_full};
use crate::db::put::now_ms;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub rank: RankMode,
    pub limit: usize,
    pub after: Option<String>, // full cursor token
    pub show: OutputFieldSelector,
    pub explain: bool,
    pub cursor_mode: CursorMode,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            rank: RankMode::Default,
            limit: 20,
            after: None,
            show: OutputFieldSelector::None,
            explain: false,
            cursor_mode: CursorMode::Full,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlannedQuery {
    pub sql: String,
    pub params: Vec<rusqlite::types::Value>,
    pub explain: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchRow {
    pub item_id: i64,
    pub path: String,
    pub data_json: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub score: Option<f64>,
}

/// Plan a search query (parse + normalize + compile).
pub fn plan_search(
    _conn: &Connection,
    schema: &Schema,
    query: &str,
    opts: &SearchOptions,
) -> Result<PlannedQuery> {
    // Parse and normalize
    let expr = parse_query(query)?;
    let normalized = normalize::normalize(expr)?;
    
    // Compile to CTEs
    let compiled = compile_to_ctes(schema, normalized, now_ms())?;
    
    // Build SQL
    let limit_plus_one = opts.limit + 1; // For has_more detection
    
    let schema_json = schema.to_json()?;
    let rank_ser = RankModeSer::from(&opts.rank);
    let expected_hash = hash_query(&schema_json, query, &rank_ser);

    let after_filter = if let Some(tok) = &opts.after {
        let pos = decode_full(tok)?;
        if pos.hash != expected_hash {
            return Err(crate::MinistoreError::Cursor("cursor does not match query/schema/rank".into()));
        }
        // We use CursorPayload.rank_value as:
        // - FTS: score
        // - Recency/default-non-fts: updated_at as f64
        // - None: None
        // Filters must match the ORDER BY in planner:
        match opts.rank {
            RankMode::None => {
                // ORDER BY i.id ASC
                // after: item_id > last_id
                // Note: uses i.id in WHERE
                Some(format!("i.id > {}", pos.payload.item_id))
            }
            RankMode::Recency | RankMode::Default | RankMode::Field(_) => {
                // ORDER BY i.updated_at DESC, i.path ASC (for non-FTS default and recency, and current Field fallback)
                let u = pos.payload.rank_value.ok_or_else(|| crate::MinistoreError::Cursor("cursor missing rank_value".into()))?;
                let updated_at = u as i64;
                // Escape single quotes in path for SQL literal safety
                let path = pos.payload.path.replace('\'', "''");
                Some(format!(
                    "(i.updated_at < {} OR (i.updated_at = {} AND i.path > '{}'))",
                    updated_at, updated_at, path
                ))
            }
        }
    } else {
        None
    };
    
    let (sql, params) = build_search_sql(
        schema,
        compiled.clone(),
        &opts.rank,
        limit_plus_one,
        after_filter,
    )?;
    
    Ok(PlannedQuery {
        sql,
        params,
        explain: compiled.explain_steps,
    })
}

/// Execute a planned query.
pub fn run_search(
    conn: &Connection,
    _schema: &Schema,
    plan: PlannedQuery,
    limit: usize,
    _after: Option<String>, // Unused now, kept for signature comp logic if any, but properly ignored in run_search
) -> Result<(Vec<SearchRow>, bool)> {
    let mut stmt = conn.prepare(&plan.sql)?;
    
    let rows: Vec<SearchRow> = stmt
        .query_map(rusqlite::params_from_iter(&plan.params), |row| {
            Ok(SearchRow {
                item_id: row.get(0)?,
                path: row.get(1)?,
                data_json: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                score: row.get(5).ok(),
            })
        })?
        .collect::<std::result::Result<_, _>>()?;
    
    // Check if there are more results
    let has_more = rows.len() > limit;
    let results = if has_more {
        rows.into_iter().take(limit).collect()
    } else {
        rows
    };
    
    Ok((results, has_more))
}

/// Convert SearchRow to ItemView.
pub fn search_row_to_item_view(row: SearchRow) -> Result<ItemView> {
    let doc: Value = serde_json::from_str(&row.data_json)?;
    
    Ok(ItemView {
        path: row.path,
        doc,
        meta: ItemMeta {
            created_at_ms: row.created_at,
            updated_at_ms: row.updated_at,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Schema;
    use crate::schema::FieldSpec;

    #[test]
    fn test_plan_search() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(1.0)));
        
        let opts = SearchOptions::default();
        
        // Need a connection for the signature, but plan_search doesn't use it
        let conn = Connection::open_in_memory().unwrap();
        
        // Use FTS text query which is a positive anchor
        let plan = plan_search(&conn, &schema, "title:test", &opts).unwrap();
        
        assert!(!plan.sql.is_empty());
        assert!(!plan.explain.is_empty());
    }
}
