use rusqlite::Connection;
use crate::{Result, Schema, ItemView, item::ItemMeta};
use crate::query::{parse_query, normalize};
use crate::query::planner::{compile_to_ctes, build_search_sql};
use crate::index::{RankMode, OutputFieldSelector, CursorMode};
use crate::cursor::{CursorPosition, CursorPayload};
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
    pub requires_fts_rank: bool,
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
    after: Option<&CursorPosition>,
) -> Result<PlannedQuery> {
    // Parse and normalize
    let expr = parse_query(query)?;
    let normalized = normalize::normalize(expr)?;
    
    // Compile to CTEs
    let compiled = compile_to_ctes(schema, normalized, now_ms())?;
    
    // Build SQL
    let limit_plus_one = opts.limit + 1; // For has_more detection

    // Build after-filter with bound params (avoid SQL injection and match ordering rules).
    // We produce a SQL fragment that references "score", "i.updated_at", "i.path", "i.id".
    let mut after_filter_sql: Option<String> = None;
    let mut after_params: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(pos) = after {
        match (&opts.rank, &pos.payload) {
            (RankMode::None, CursorPayload::None { item_id }) => {
                after_filter_sql = Some("i.id > ?".to_string());
                after_params.push((*item_id).into());
            }
            // Default w/ FTS: ORDER BY score DESC, i.id ASC
            (RankMode::Default, CursorPayload::Fts { score, item_id }) => {
                after_filter_sql = Some("(score < ? OR (score = ? AND i.id > ?))".to_string());
                after_params.push((*score).into());
                after_params.push((*score).into());
                after_params.push((*item_id).into());
            }
            // Recency ordering and Default (non-FTS fallback): ORDER BY updated_at DESC, path ASC
            (RankMode::Recency, CursorPayload::Recency { updated_at_ms, path })
            | (RankMode::Default, CursorPayload::Recency { updated_at_ms, path }) => {
                after_filter_sql = Some("(i.updated_at < ? OR (i.updated_at = ? AND i.path > ?))".to_string());
                after_params.push((*updated_at_ms).into());
                after_params.push((*updated_at_ms).into());
                after_params.push(path.clone().into());
            }
            // Field ordering: ORDER BY score DESC, updated_at DESC, path ASC (score is rank_value)
            (RankMode::Field(_), CursorPayload::Field { rank_value, updated_at_ms, path, .. }) => {
                after_filter_sql = Some(
                    "(score < ? OR (score = ? AND (i.updated_at < ? OR (i.updated_at = ? AND i.path > ?))))"
                        .to_string()
                );
                after_params.push((*rank_value).into());
                after_params.push((*rank_value).into());
                after_params.push((*updated_at_ms).into());
                after_params.push((*updated_at_ms).into());
                after_params.push(path.clone().into());
            }
            _ => {
                return Err(crate::MinistoreError::Cursor("cursor payload does not match rank mode".into()));
            }
        }
    }
    
    let (mut sql, mut params) = build_search_sql(
        schema,
        compiled.clone(),
        &opts.rank,
        limit_plus_one,
        after_filter_sql,
    )?;

    // Append field ranking param (planner expects it as last param).
    if let RankMode::Field(field_name) = &opts.rank {
        params.push(field_name.clone().into());
    }
    // Append after-filter params after everything else (they use anonymous '?' placeholders).
    // build_search_sql currently inserts after_filter as "AND (<fragment>)" where fragment contains '?'
    // so these must be in order after compiled params (+rank field param).
    params.extend(after_params);
    
    Ok(PlannedQuery {
        sql,
        params,
        explain: compiled.explain_steps,
        requires_fts_rank: matches!(opts.rank, RankMode::Default) && compiled.requires_fts_join,
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
        let plan = plan_search(&conn, &schema, "title:test", &opts, None).unwrap();
        
        assert!(!plan.sql.is_empty());
        assert!(!plan.explain.is_empty());
    }
}
