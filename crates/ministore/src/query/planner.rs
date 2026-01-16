use crate::{Result, Schema, MinistoreError, FieldType};
use crate::query::ast::*;
use crate::index::RankMode;

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
                let sql = "SELECT item_id FROM field_present WHERE field = ?1".to_string();
                self.params.push(field.clone().into());
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
                    self.params.push(format!("{}%", prefix).into());
                    "SELECT id AS item_id FROM items WHERE path LIKE ?1".to_string()
                } else {
                    self.params.push(pattern.clone().into());
                    "SELECT id AS item_id FROM items WHERE path GLOB ?1".to_string()
                };
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("PATH {}", pattern));
                Ok(result_name)
            }
            
            Predicate::Keyword { field, pattern, kind } => {
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;

                // If schema says this is a TEXT field, treat field:term as FTS query.
                if spec.field_type == FieldType::Text {
                    return self.compile_predicate(Predicate::Text { field: Some(field), fts: pattern });
                }
                // Bool fields: accept true/false via field:...
                if spec.field_type == FieldType::Bool && (pattern == "true" || pattern == "false") {
                    return self.compile_predicate(Predicate::Bool { field, value: pattern == "true" });
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
                        self.params.push(field.clone().into());
                        self.params.push(pattern.clone().into());
                        "SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = ?1 AND d.value = ?2".to_string()
                    }
                    KeywordPatternKind::Prefix => {
                        let prefix = pattern.trim_end_matches('*').to_string();
                        self.params.push(field.clone().into());
                        self.params.push(format!("{}%", prefix).into());
                        "SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = ?1 AND d.value LIKE ?2".to_string()
                    }
                    KeywordPatternKind::Contains => {
                        let inner = pattern.trim_matches('*').to_string();
                        self.params.push(field.clone().into());
                        self.params.push(format!("%{}%", inner).into());
                        "SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = ?1 AND d.value LIKE ?2".to_string()
                    }
                    KeywordPatternKind::Glob => {
                        self.params.push(field.clone().into());
                        self.params.push(pattern.clone().into());
                        "SELECT p.item_id \
                         FROM kw_dict d \
                         JOIN kw_postings p ON p.value_id = d.id \
                         WHERE d.field = ?1 AND d.value GLOB ?2".to_string()
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
                let match_str = if let Some(f) = field {
                    // validate text field exists
                    let spec = self.schema.get(&f).ok_or_else(|| MinistoreError::UnknownField(f.clone()))?;
                    if spec.field_type != FieldType::Text {
                        return Err(MinistoreError::TypeMismatch {
                            field: f,
                            message: "FTS predicate used on non-text field".into(),
                        });
                    }
                    format!("{}:{}", f, fts)
                } else {
                    let cols = self.schema.text_fields_in_order();
                    if cols.is_empty() {
                        return Err(MinistoreError::QueryRejected("no text fields in schema for bare text query".into()));
                    }
                    let parts: Vec<String> = cols.into_iter().map(|(c, _)| format!("{}:{}", c, fts)).collect();
                    format!("({})", parts.join(" OR "))
                };
                self.params.push(match_str.into());
                let sql = "SELECT rowid AS item_id FROM search WHERE search MATCH ?1".to_string();
                
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
                self.params.push(field.clone().into());
                self.params.push(value.into());
                let sql = format!("SELECT item_id FROM field_number WHERE field = ?1 AND value {} ?2", op_str);
                
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
                self.params.push(field.clone().into());
                self.params.push(lo.into());
                self.params.push(hi.into());
                let sql = "SELECT item_id FROM field_number WHERE field = ?1 AND value >= ?2 AND value <= ?3".to_string();
                
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
                    self.params.push(epoch_ms.into());
                    let sql = format!("SELECT id AS item_id FROM items WHERE {} {} ?1", col, op_str);
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
                self.params.push(field.clone().into());
                self.params.push(epoch_ms.into());
                let sql = format!("SELECT item_id FROM field_date WHERE field = ?1 AND value {} ?2", op_str);
                
                self.ctes.push(Cte {
                    name: result_name.clone(),
                    sql,
                });
                self.explain.push(format!("DATE {}{}{}", field, op_str, epoch_ms));
                Ok(result_name)
            }

            Predicate::DateRangeAbs { field, lo_ms, hi_ms } => {
                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
                if spec.field_type != FieldType::Date {
                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
                }
                let result_name = self.next_cte_name();
                self.params.push(field.clone().into());
                self.params.push(lo_ms.into());
                self.params.push(hi_ms.into());
                let sql = "SELECT item_id FROM field_date WHERE field = ?1 AND value >= ?2 AND value <= ?3".to_string();

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
                    self.params.push(target_ms.into());
                    let sql = format!("SELECT id AS item_id FROM items WHERE {} {} ?1", col, op_str);
                    
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
                    self.params.push(field.clone().into());
                    self.params.push(target_ms.into());
                    let sql = format!("SELECT item_id FROM field_date WHERE field = ?1 AND value {} ?2", op_str);
                    
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
                self.params.push(field.clone().into());
                self.params.push((int_val as i64).into());
                let sql = "SELECT item_id FROM field_bool WHERE field = ?1 AND value = ?2".to_string();
                
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
    
    let with_clause = if sql_parts.is_empty() {
        String::new()
    } else {
        format!("WITH {} ", sql_parts.join(", "))
    };
    
    // Build main SELECT with proper ranking
    let select_cols = "i.id, i.path, i.data_json, i.created_at, i.updated_at";
    
    // Always return a 6th column called "score" (REAL or NULL).
    let (order_clause, score_expr, fts_join) = if matches!(rank, RankMode::Default) && compiled.requires_fts_join {
        // Weighted BM25: score = -bm25(search, w1, w2, ...  )
        // FTS5's bm25() is "smaller is better", so negate it
        let text_fields = schema.text_fields_in_order();
        let weights: Vec<String> = text_fields.iter()
            .map(|(_, weight)| weight.to_string())
            .collect();
        let weights_str = weights.join(", ");
        
        let score_col = format!("(-bm25(search, {}))", weights_str);
        (
            String::from("ORDER BY score DESC, i.id ASC"),
            score_col,
            String::from("JOIN search ON search.rowid = i.id")
        )
    } else {
        // Other ranking modes
        match rank {
            RankMode::Recency => (
                String::from("ORDER BY i.updated_at DESC, i.path ASC"),
                String::from("CAST(i.updated_at AS REAL)"),
                String::new()
            ),
            RankMode::Field(_field_name) => {
                // Field-based ranking - simplified for now
                // Field ranking is handled in outer layer (db/search) by joining field_number/field_date
                // Here we fall back to recency ordering and a NULL score.
                (
                    String::from("ORDER BY i.updated_at DESC, i.path ASC"),
                    String::from("CAST(i.updated_at AS REAL)"),
                    String::new()
                )
            },
            RankMode::None => (
                String::from("ORDER BY i.id ASC"),
                String::from("NULL"),
                String::new()
            ),
            RankMode::Default => {
                // Default without FTS - fallback to recency
                (
                    String::from("ORDER BY i.updated_at DESC, i.path ASC"),
                    String::from("CAST(i.updated_at AS REAL)"),
                    String::new()
                )
            }
        }
    };
    
    let after_clause = after_filter.as_ref().map(|f| format!("AND ({})", f)).unwrap_or_default();
    
    let sql = format!(
        "{}SELECT {}, {} AS score FROM items i {} \
         JOIN {} r ON r.item_id = i.id \
         WHERE 1=1 {} {} LIMIT {}",
        with_clause,
        select_cols,
        score_expr,
        fts_join,
        compiled.result_cte_name,
        after_clause,
        order_clause,
        limit_plus_one
    );
    
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
