use crate::{Result, Schema, MinistoreError, FieldType};
use crate::query::ast::*;
use crate::index::RankMode;
use chrono::{NaiveDate, DateTime};

fn parse_date_to_epoch_ms(s: &str) -> Result<i64> {
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    Err(MinistoreError::QueryParse(format!("invalid date format: {}", s)))
}

/// Quote an FTS term if it contains spaces, quotes, or other special characters.
/// This is necessary because FTS5 MATCH syntax treats spaces as term separators.
fn quote_fts_term(term: &str) -> String {
    // If term contains whitespace, quotes, or FTS operators, wrap in double quotes and escape internal quotes
    if term.chars().any(|c| c.is_whitespace() || c == '"' || c == ':' || c == '*' || c == '^' || c == '(' || c == ')') {
        // Escape internal double quotes by doubling them
        let escaped = term.replace('"', "\"\"" );
        format!("\"{}\"" , escaped)
    } else {
        term.to_string()
    }
}

#[derive(Debug, Clone)]
pub struct CompileOutput {
    pub ctes: Vec<Cte>,
    pub result_cte_name: String,
    pub fts_match: Option<String>,
    pub params: Vec<rusqlite::types::Value>,
    pub explain_steps: Vec<String>,
    pub requires_fts_join: bool,
}

#[derive(Debug, Clone)]
pub struct Cte {
    pub name: String,
    pub sql: String,
}

/// Compile expression to CTEs.
pub fn compile_to_ctes(schema: &Schema, expr: Expr, now_ms: i64) -> Result<CompileOutput> {
    let mut compiler = Compiler::new(schema, now_ms);
    let result_name = compiler.compile_expr(expr)?;
    
    Ok(CompileOutput {
        ctes: compiler.ctes,
        result_cte_name: result_name,
        fts_match: compiler.fts_match,
        params: compiler.params,
        explain_steps: compiler.explain,
        requires_fts_join: compiler.requires_fts_join,
    })
}

struct Compiler<'a> {
    schema: &'a Schema,
    now_ms: i64,
    ctes: Vec<Cte>,
    params: Vec<rusqlite::types::Value>,
    explain: Vec<String>,
    cte_counter: usize,
    fts_match: Option<String>,
    requires_fts_join: bool,
}

impl<'a> Compiler<'a> {
    fn new(schema: &'a Schema, now_ms: i64) -> Self {
        Self {
            schema,
            now_ms,
            ctes: Vec::new(),
            params: Vec::new(),
            explain: Vec::new(),
            cte_counter: 0,
            fts_match: None,
            requires_fts_join: false,
        }
    }

    fn next_cte_name(&mut self) -> String {
        let name = format!("cte_{}", self.cte_counter);
        self.cte_counter += 1;
        name
    }

    fn push_param(&mut self, v: rusqlite::types::Value) -> String {
        self.params.push(v);
        format!("?{}", self.params.len())
    }

    fn compile_expr(&mut self, expr: Expr) -> Result<String> {
        match expr {
            Expr::And(left, right) => {
                let left_name = self.compile_expr(*left)?;
                let right_name = self.compile_expr(*right)?;
                let result_name = self.next_cte_name();
                
                let sql = format!(
                    "SELECT item_id FROM {} INTERSECT SELECT item_id FROM {}",
                    left_name, right_name
                );
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("INTERSECT {} AND {}", left_name, right_name));
                
                Ok(result_name)
            }
            Expr::Or(left, right) => {
                let left_name = self.compile_expr(*left)?;
                let right_name = self.compile_expr(*right)?;
                let result_name = self.next_cte_name();
                
                let sql = format!(
                    "SELECT item_id FROM {} UNION SELECT item_id FROM {}",
                    left_name, right_name
                );
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("UNION {} OR {}", left_name, right_name));
                
                Ok(result_name)
            }
            Expr::Not(inner) => {
                let inner_name = self.compile_expr(*inner)?;
                let result_name = self.next_cte_name();
                
                // Complement is expressed with EXCEPT (NULL-safe, matches design intent)
                let sql = format!(
                    "SELECT id AS item_id FROM items EXCEPT SELECT item_id FROM {}",
                    inner_name
                );
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("EXCEPT NOT {}", inner_name));
                
                Ok(result_name)
            }
            Expr::Pred(pred) => self.compile_predicate(pred),
        }
    }

    fn compile_predicate(&mut self, pred: Predicate) -> Result<String> {
        match pred {
            Predicate::Has { field } => {
                if !self.schema.has_field(&field) {
                    return Err(MinistoreError::UnknownField(field));
                }
                let result_name = self.next_cte_name();
                let p_field = self.push_param(field.clone().into());
                let sql = format!("SELECT item_id FROM field_present WHERE field = {}", p_field);
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("HAS {}", field));
                Ok(result_name)
            }
            
            Predicate::PathGlob { pattern } => {
                let result_name = self.next_cte_name();
                // Prefix-only optimization: "/docs/*" => LIKE "/docs/%"
                // Otherwise: use GLOB
                let sql = if pattern.ends_with('*') && !pattern[..pattern.len()-1].contains('*') && !pattern.contains('?') {
                    let prefix = pattern.trim_end_matches('*').to_string();
                    let p = self.push_param(format!("{}%", prefix).into());
                    format!("SELECT id AS item_id FROM items WHERE path LIKE {}", p)
                } else {
                    let p = self.push_param(pattern.clone().into());
                    format!("SELECT id AS item_id FROM items WHERE path GLOB {}", p)
                };
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("PATH {}", pattern));
                Ok(result_name)
            }
            
            Predicate::Keyword { field, pattern, kind } => {
                // Implicit created/updated support for equality: created:2024-01-01
                if field == "created" || field == "updated" {
                    if kind != KeywordPatternKind::Exact {
                        return Err(MinistoreError::TypeMismatch {
                            field,
                            message: "wildcards not supported for implicit date fields".into(),
                        });
                    }
                    let epoch_ms = parse_date_to_epoch_ms(&pattern)?;
                    let result_name = self.next_cte_name();
                    let col = if field == "created" { "created_at" } else { "updated_at" };
                    let p = self.push_param(epoch_ms.into());
                    let sql = format!("SELECT id AS item_id FROM items WHERE {} = {}", col, p);
                    self.ctes.push(Cte { name: result_name.clone(), sql });
                    self.explain.push(format!("IMPLICIT DATE {}:{}", field, pattern));
                    return Ok(result_name);
                }

                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;

                // If schema says this is a TEXT field, treat field:term as FTS query.
                if spec.field_type == FieldType::Text {
                    return self.compile_predicate(Predicate::Text { field: Some(field), fts: pattern });
                }
                // Bool fields: accept true/false via field:...
                if spec.field_type == FieldType::Bool && (pattern == "true" || pattern == "false") {
                    return self.compile_predicate(Predicate::Bool { field, value: pattern == "true" });
                }
                // Date fields: support equality via field:YYYY-MM-DD (exact only)
                if spec.field_type == FieldType::Date {
                    if kind != KeywordPatternKind::Exact {
                        return Err(MinistoreError::TypeMismatch {
                            field,
                            message: "wildcards not supported for date fields; use comparisons".into(),
                        });
                    }
                    let epoch_ms = parse_date_to_epoch_ms(&pattern)?;
                    return self.compile_predicate(Predicate::DateCmpAbs {
                        field,
                        op: CmpOp::Eq,
                        epoch_ms,
                    });
                }
                if spec.field_type != FieldType::Keyword {
                    return Err(MinistoreError::TypeMismatch {
                        field: field.clone(),
                        message: format!("predicate is keyword-style but schema type is {:?}", spec.field_type),
                    });
                }

                let result_name = self.next_cte_name();

                // All keyword paths use dict + postings; filter via dict on (field,value)
                // Use bound reminders:
                //   ?1 = field
                //   ?2 = pattern value / like / glob
                let sql = match kind {
                    KeywordPatternKind::Exact => {
                        let p_field = self.push_param(field.clone().into());
                        let p_val = self.push_param(pattern.clone().into());
                        format!("SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = {} AND d.value = {}", p_field, p_val)
                    }
                    KeywordPatternKind::Prefix => {
                        let prefix = pattern.trim_end_matches('*').to_string();
                        let p_field = self.push_param(field.clone().into());
                        let p_val = self.push_param(format!("{}%", prefix).into());
                        format!("SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = {} AND d.value LIKE {}", p_field, p_val)
                    }
                    KeywordPatternKind::Contains => {
                        let inner = pattern.trim_matches('*').to_string();
                        let p_field = self.push_param(field.clone().into());
                        let p_val = self.push_param(format!("%{}%", inner).into());
                        format!("SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = {} AND d.value LIKE {}", p_field, p_val)
                    }
                    KeywordPatternKind::Glob => {
                        let p_field = self.push_param(field.clone().into());
                        let p_val = self.push_param(pattern.clone().into());
                        format!("SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = {} AND d.value GLOB {}", p_field, p_val)
                    }
                };
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("KEYWORD {}:{}", field, pattern));
                Ok(result_name)
            }
            
            Predicate::Text { field, fts } => {
                self.requires_fts_join = true;
                // We store only for debug; planner builds proper MATCH string below.
                self.fts_match = Some(fts.clone());
                
                let result_name = self.next_cte_name();
                // Build MATCH string:
                // - field Some => "field:term"
                // - field None => "(col1:term OR col2:term ...)"
                let quoted_fts = quote_fts_term(&fts);
                let match_str = if let Some(f) = field {
                    // validate text field exists
                    let spec = self.schema.get(&f).ok_or_else(|| MinistoreError::UnknownField(f.clone()))?;
                    if spec.field_type != FieldType::Text {
                        return Err(MinistoreError::TypeMismatch {
                            field: f,
                            message: "FTS predicate used on non-text field".into(),
                        });
                    }
                    format!("{}:{}", f, quoted_fts)
                } else {
                    let cols = self.schema.text_fields_in_order();
                    if cols.is_empty() {
                        return Err(MinistoreError::QueryRejected("no text fields in schema for bare text query".into()));
                    }
                    let parts: Vec<String> = cols.into_iter().map(|(c, _)| format!("{}:{}", c, quoted_fts)).collect();
                    format!("({})", parts.join(" OR "))
                };
                let p_match = self.push_param(match_str.into());

                let sql = format!("SELECT rowid AS item_id FROM search WHERE search MATCH {}", p_match);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("FTS {}", fts));
                Ok(result_name)
            }
            
            Predicate::NumberCmp { field, op, value } => {
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Number {
                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected number field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                let op_str = match op {
                    CmpOp::Eq => "=",
                    CmpOp::Gt => ">",
                    CmpOp::Gte => ">=",
                    CmpOp::Lt => "<",
                    CmpOp::Lte => "<=",
                };
                let p_field = self.push_param(field.clone().into());
                let p_val = self.push_param(value.into());
                let sql = format!("SELECT item_id FROM field_number WHERE field = {} AND value {} {}", p_field, op_str, p_val);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("NUMBER {}{}{}", field, op_str, value));
                Ok(result_name)
            }
            
            Predicate::NumberRange { field, lo, hi } => {
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Number {
                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected number field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                let p_field = self.push_param(field.clone().into());
                let p_lo = self.push_param(lo.into());
                let p_hi = self.push_param(hi.into());
                let sql = format!("SELECT item_id FROM field_number WHERE field = {} AND value >= {} AND value <= {}", p_field, p_lo, p_hi);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("NUMBER {}:{}..{}", field, lo, hi));
                Ok(result_name)
            }
            
            Predicate::DateCmpAbs { field, op, epoch_ms } => {
                // implicit created/updated => items table columns
                if field == "created" || field == "updated" {
                    let result_name = self.next_cte_name();
                    let col = if field == "created" { "created_at" } else { "updated_at" };
                    let op_str = match op {
                        CmpOp::Eq => "=",
                        CmpOp::Gt => ">",
                        CmpOp::Gte => ">=",
                        CmpOp::Lt => "<",
                        CmpOp::Lte => "<=",
                    };
                    let p = self.push_param(epoch_ms.into());
                    let sql = format!("SELECT id AS item_id FROM items WHERE {} {} {}", col, op_str, p);
                    self.ctes.push(Cte { name: result_name.clone(), sql });
                    self.explain.push(format!("DATE {}{}{}", field, op_str, epoch_ms));
                    return Ok(result_name);
                }
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Date {
                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                let op_str = match op {
                    CmpOp::Eq => "=",
                    CmpOp::Gt => ">",
                    CmpOp::Gte => ">=",
                    CmpOp::Lt => "<",
                    CmpOp::Lte => "<=",
                };
                let p_field = self.push_param(field.clone().into());
                let p_val = self.push_param(epoch_ms.into());
                let sql = format!("SELECT item_id FROM field_date WHERE field = {} AND value {} {}", p_field, op_str, p_val);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("DATE {}{}{}", field, op_str, epoch_ms));
                Ok(result_name)
            }

            Predicate::DateRangeAbs { field, lo_ms, hi_ms } => {
                // implicit created/updated ranges compile to items table
                if field == "created" || field == "updated" {
                    let result_name = self.next_cte_name();
                    let col = if field == "created" { "created_at" } else { "updated_at" };
                    let p_lo = self.push_param(lo_ms.into());
                    let p_hi = self.push_param(hi_ms.into());
                    let sql = format!(
                        "SELECT id AS item_id FROM items WHERE {} >= {} AND {} <= {}",
                        col, p_lo, col, p_hi
                    );
                    self.ctes.push(Cte { name: result_name.clone(), sql });
                    self.explain.push(format!("IMPLICIT DATE RANGE {}:{}..{}", field, lo_ms, hi_ms));
                    return Ok(result_name);
                }

                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Date {
                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                let p_field = self.push_param(field.clone().into());
                let p_lo = self.push_param(lo_ms.into());
                let p_hi = self.push_param(hi_ms.into());
                let sql = format!("SELECT item_id FROM field_date WHERE field = {} AND value >= {} AND value <= {}", p_field, p_lo, p_hi);

                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("DATE {}:{}..{}", field, lo_ms, hi_ms));
                Ok(result_name)
            }
            
            Predicate::DateCmpRel { field, op, amount, unit } => {
                // Convert duration to ms (approx for M/Y)
                let duration_ms: i64 = match unit {
                    RelUnit::H => amount * 3600 * 1000,
                    RelUnit::D => amount * 24 * 3600 * 1000,
                    RelUnit::W => amount * 7 * 24 * 3600 * 1000,
                    RelUnit::M => amount * 30 * 24 * 3600 * 1000, // Approximate
                    RelUnit::Y => amount * 365 * 24 * 3600 * 1000, // Approximate
                };

                // Relative semantics:
                // - created/updated: interpret as "age"
                //     created:<7d => created_at >= now - 7d
                //     updated:>30d => updated_at <= now - 30d
                // - schema date fields: interpret as "offset from now"
                //     due:<7d => due < now + 7d
                let is_implicit = field == "created" || field == "updated";
                let target_ms = if is_implicit {
                    self.now_ms - duration_ms
                } else {
                    self.now_ms + duration_ms
                };
                
                // Use items table for implicit fields
                if is_implicit {
                    let result_name = self.next_cte_name();
                    let col = if field == "created" { "created_at" } else { "updated_at" };
                    // Age semantics mapping:
                    //   <Nd => >= now-Nd
                    //   >Nd => <= now-Nd
                    let mapped_op = match op {
                        CmpOp::Lt => CmpOp::Gte,
                        CmpOp::Lte => CmpOp::Gte,
                        CmpOp::Gt => CmpOp::Lte,
                        CmpOp::Gte => CmpOp::Lte,
                        CmpOp::Eq => CmpOp::Eq,
                    };
                    let op_str = match mapped_op {
                        CmpOp::Eq => "=",
                        CmpOp::Gt => ">",
                        CmpOp::Gte => ">=",
                        CmpOp::Lt => "<",
                        CmpOp::Lte => "<=",
                    };
                    let p = self.push_param(target_ms.into());
                    let sql = format!("SELECT id AS item_id FROM items WHERE {} {} {}", col, op_str, p);
                    
                    self.ctes.push(Cte {
                        name: result_name.clone(),
                        sql,
                    });
                    self.explain.push(format!("DATE(rel-age) {}{}{}{:?}", field, op_str, amount, unit));
                    Ok(result_name)
                } else {
                    // For schema-defined date fields
                    let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                    if spec.field_type != FieldType::Date {
                        return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
                    }
                    let result_name = self.next_cte_name();
                    let op_str = match op {
                        CmpOp::Eq => "=",
                        CmpOp::Gt => ">",
                        CmpOp::Gte => ">=",
                        CmpOp::Lt => "<",
                        CmpOp::Lte => "<=",
                    };
                    let p_field = self.push_param(field.clone().into());
                    let p_val = self.push_param(target_ms.into());
                    let sql = format!("SELECT item_id FROM field_date WHERE field = {} AND value {} {}", p_field, op_str, p_val);
                    
                    self.ctes.push(Cte {
                        name: result_name.clone(),
                        sql,
                    });
                    self.explain.push(format!("DATE(rel) {}{}{}{:?}", field, op_str, amount, unit));
                    Ok(result_name)
                }
            }
            
            Predicate::Bool { field, value } => {
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Bool {
                     return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected bool field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                let int_val = if value { 1 } else { 0 };
                let p_field = self.push_param(field.clone().into());
                let p_val = self.push_param((int_val as i64).into());
                let sql = format!("SELECT item_id FROM field_bool WHERE field = {} AND value = {}", p_field, p_val);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("BOOL {}:{}", field, value));
                Ok(result_name)
            }
        }
    }
}

/// Build final SELECT given compiled CTEs and rank mode.
pub fn build_search_sql(
    schema: &Schema,
    compiled: CompileOutput,
    rank: &RankMode,
    limit_plus_one: usize,
    after_filter: Option<String>,
) -> Result<(String, Vec<rusqlite::types::Value>)> {
    let mut sql_parts = Vec::new();
    
    // Build CTEs
    for cte in &compiled.ctes {
        sql_parts.push(format!("{} AS ({})", cte.name, cte.sql));
    }

    // Optional CTE for field ranking
    let mut field_rank_cte_name: Option<String> = None;
    if let RankMode::Field(field_name) = rank {
        let spec = schema.get(field_name).ok_or_else(|| MinistoreError::UnknownField(field_name.clone()))?;
        let (table, value_col) = match spec.field_type {
            FieldType::Number => ("field_number", "value"),
            FieldType::Date => ("field_date", "value"),
            _ => {
                return Err(MinistoreError::TypeMismatch {
                    field: field_name.clone(),
                    message: "rank field must be number or date".into(),
                });
            }
        };
        // Append param for the field name (must be appended AFTER compiled.params).
        // We'll do it at the end, outside of this function, by returning SQL that uses ?{n}.
        // So here we inject a placeholder that assumes it's the next parameter index.
        // We compute it based on compiled.params.len() + 1.
        let p_field = format!("?{}", compiled.params.len() + 1);
        let cte_name = "rank_field".to_string();
        let cte_sql = format!(
            "SELECT item_id, MAX({}) AS rank_value FROM {} WHERE field = {} GROUP BY item_id",
            value_col, table, p_field
        );
        sql_parts.push(format!("{} AS ({})", cte_name, cte_sql));
        field_rank_cte_name = Some(cte_name);
    }
    
    let with_clause = if sql_parts.is_empty() {
        String::new()
    } else {
        format!("WITH {} ", sql_parts.join(", "))
    };
    
    // Build main SELECT with proper ranking
    // Inner query aliases columns so outer query can reference them by name
    let select_cols_inner = "i.id AS item_id, i.path AS path, i.data_json AS data_json, i.created_at AS created_at, i.updated_at AS updated_at";

    // Always return a 6th column called "score" (REAL or NULL).
    // Order clauses use unqualified names since they apply to the derived table output
    let (order_clause, score_expr, fts_join, extra_join) = if matches!(rank, RankMode::Default) && compiled.requires_fts_join {
        // Weighted BM25: score = -bm25(search, w1, w2, ...  )
        // FTS5's bm25() is "smaller is better", so negate it
        let text_fields = schema.text_fields_in_order();
        let weights: Vec<String> = text_fields.iter()
            .map(|(_, weight)| weight.to_string())
            .collect();
        let weights_str = weights.join(", ");
        
        let score_col = format!("(-bm25(search, {}))", weights_str);
        (
            String::from("ORDER BY score DESC, item_id ASC"),
            score_col,
            String::from("JOIN search ON search.rowid = i.id"),
            String::new()
        )
    } else {
        // Other ranking modes
        match rank {
            RankMode::Recency => (
                String::from("ORDER BY updated_at DESC, path ASC"),
                String::from("CAST(i.updated_at AS REAL)"),
                String::new(),
                String::new()
            ),
            RankMode::Field(_field_name) => {
                let rf = field_rank_cte_name.as_ref().expect("rank_field cte");
                (
                    String::from("ORDER BY score DESC, updated_at DESC, path ASC"),
                    format!("CAST({}.rank_value AS REAL)", rf),
                    String::new(),
                    format!("JOIN {} ON {}.item_id = i.id", rf, rf)
                )
            },
            RankMode::None => (
                String::from("ORDER BY item_id ASC"),
                String::from("NULL"),
                String::new(),
                String::new()
            ),
            RankMode::Default => {
                // Default without FTS - fallback to recency
                (
                    String::from("ORDER BY updated_at DESC, path ASC"),
                    String::from("CAST(i.updated_at AS REAL)"),
                    String::new(),
                    String::new()
                )
            }
        }
    };
    
    let after_where = after_filter
        .as_ref()
        .map(|f| format!("AND ({})", f))
        .unwrap_or_default();

    // Inner query computes score, outer query applies after-filter and ordering.
    // This makes `score` usable in WHERE (via derived table columns).
    let sql = format!(
        "{with_clause}
         SELECT item_id, path, data_json, created_at, updated_at, score
         FROM (
           SELECT {select_cols_inner}, {score_expr} AS score
           FROM items i
           {fts_join}
           {extra_join}
           JOIN {result} r ON r.item_id = i.id
         ) q
         WHERE 1=1 {after_where}
         {order_clause}
         LIMIT {limit_plus_one}",
        with_clause = with_clause,
        select_cols_inner = select_cols_inner,
        score_expr = score_expr,
        fts_join = fts_join,
        extra_join = extra_join,
        result = compiled.result_cte_name,
        after_where = after_where,
        order_clause = order_clause,
        limit_plus_one = limit_plus_one,
    );
    
    // If RankMode::Field, caller MUST append the extra field-name param to compiled.params.
    Ok((sql, compiled.params))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse_query;
    use crate::schema::FieldSpec;

    #[test]
    fn test_compile_simple_has() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(1.0)));
        
        let expr = parse_query("has:title").unwrap();
        let output = compile_to_ctes(&schema, expr, 0).unwrap();
        
        assert!(output.ctes.len() > 0);
        assert!(output.explain_steps.len() > 0);
    }

    #[test]
    fn test_compile_and() {
        let mut schema = Schema::new();
        schema.add_field("title", FieldSpec::text(Some(1.0)));
        schema.add_field("active", FieldSpec::bool());
        
        let expr = parse_query("has:title AND active:true").unwrap();
        let output = compile_to_ctes(&schema, expr, 0).unwrap();
        
        // Should have CTEs for each predicate plus the INTERSECT
        assert!(output.ctes.len() >= 3);
    }

    #[test]
    fn test_compile_keyword() {
        let mut schema = Schema::new();
        schema.add_field("tag", FieldSpec::keyword(false));
        
        let expr = parse_query("tag:rust").unwrap();
        let output = compile_to_ctes(&schema, expr, 0).unwrap();
        
        assert!(output.ctes.len() > 0);
    }
}
