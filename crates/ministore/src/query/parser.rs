use crate::query::ast::*;
use crate::query::lexer::{lex, Tok};
use crate::{MinistoreError, Result};
use chrono::{NaiveDate, DateTime};

/// Parse a query string into an AST.
pub fn parse_query(input: &str) -> Result<Expr> {
    let tokens = lex(input)?;
    let mut parser = Parser::new(tokens);
    parser.parse_expr()
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Tok>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;

        while self.match_tok(&Tok::Or) {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_not()?;

        while self.match_tok(&Tok::And) {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }

        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr> {
        if self.match_tok(&Tok::Not) {
            self.advance();

            // Shorthand: !archived   => archived:false
            // Only when next token is an identifier AND not followed by ':' or a comparison op.
            if let Some(Tok::Ident(field)) = self.current() {
                let field_name = field.clone();
                // Lookahead token after the ident
                let next = self.tokens.get(self.pos + 1);
                let is_fielded = matches!(next, Some(Tok::Colon | Tok::Gt | Tok::Gte | Tok::Lt | Tok::Lte));
                if !is_fielded {
                    self.advance(); // consume ident
                    return Ok(Expr::Pred(Predicate::Bool {
                        field: field_name,
                        value: false,
                    }));
                }
            }

            let inner = self.parse_not()?;
            Ok(Expr::Not(Box::new(inner)))
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        // Parenthesized expression
        if self.match_tok(&Tok::LParen) {
            self.advance();
            let expr = self.parse_expr()?;
            self.expect(&Tok::RParen)?;
            return Ok(expr);
        }

        // Predicate
        Ok(Expr::Pred(self.parse_predicate()?))
    }

    fn parse_predicate(&mut self) -> Result<Predicate> {
        // A predicate starts with either an Ident or a String (quoted).
        // If it's not followed by ':' or a comparison op, it is a bare-text term (FTS across all text fields).
        let first = match self.current() {
            Some(Tok::Ident(s)) => s.clone(),
            Some(Tok::String(s)) => s.clone(),
            _ => return Err(MinistoreError::QueryParse("expected term".into())),
        };
        self.advance();

        // field:value or has:value
        if self.match_tok(&Tok::Colon) {
            self.advance();
            // Special handling for has:<field>
            if first == "has" {
                let f = self.expect_string_or_ident()?;
                return Ok(Predicate::Has { field: f });
            }
            return self.parse_field_predicate(&first);
        }

        // field comparisons: priority>5, due<2024-01-01, created<7d
        if let Some(tok) = self.current() {
            match tok {
                Tok::Gt | Tok::Gte | Tok::Lt | Tok::Lte => {
                    return self.parse_comparison(&first);
                }
                Tok::DotDot => {
                    return Err(MinistoreError::QueryParse("range requires field:start..end notation".into()));
                }
                _ => {}
            }
        }

        // Bare term => full-text across all text fields
        Ok(Predicate::Text {
            field: None,
            fts: first,
        })
    }

    fn parse_field_predicate(&mut self, field: &str) -> Result<Predicate> {
        // Special handling for path field
        if field == "path" {
            let pattern = self.expect_string_or_ident()?;
            return Ok(Predicate::PathGlob { pattern });
        }

        // Get value
        match self.current() {
            Some(Tok::String(s)) | Some(Tok::Ident(s)) => {
                let value = s.clone();
                self.advance();

                // Support date ranges: field:2024-01-01..2024-06-30
                // (This will later be validated by planner against schema type.)
                if self.match_tok(&Tok::DotDot) {
                    self.advance();
                    let hi_s = self.expect_string_or_ident()?;
                    let lo_ms = parse_date_to_epoch_ms(&value)?;
                    let hi_ms = parse_date_to_epoch_ms(&hi_s)?;
                    return Ok(Predicate::DateRangeAbs {
                        field: field.to_string(),
                        lo_ms,
                        hi_ms,
                    });
                }

                // NOTE: We do NOT decide keyword vs text here.
                // Planner will look at schema type:
                // - if field is Text => compile to FTS MATCH
                // - if field is Keyword => kw_dict/kw_postings
                // - if field is Bool and value is true/false => bool compare
                let kind = classify_keyword_pattern(&value);
                Ok(Predicate::Keyword {
                    field: field.to_string(),
                    pattern: value,
                    kind,
                })
            }
            Some(Tok::Number(n)) => {
                let val = *n;
                self.advance();
                
                // Check for range (..)
                if self.match_tok(&Tok::DotDot) {
                    self.advance();
                    let hi = self.expect_number()?;
                    Ok(Predicate::NumberRange {
                        field: field.to_string(),
                        lo: val,
                        hi,
                    })
                } else {
                    // Single number as equality check
                    Ok(Predicate::NumberCmp {
                        field: field.to_string(),
                        op: CmpOp::Eq,
                        value: val,
                    })
                }
            }
            _ => Err(MinistoreError::QueryParse(format!(
                "expected value after '{}:'", field
            ))),
        }
    }

    fn parse_comparison(&mut self, field: &str) -> Result<Predicate> {
        let op = match self.current() {
            Some(Tok::Gt) => CmpOp::Gt,
            Some(Tok::Gte) => CmpOp::Gte,
            Some(Tok::Lt) => CmpOp::Lt,
            Some(Tok::Lte) => CmpOp::Lte,
            _ => return Err(MinistoreError::QueryParse("expected comparison operator".into())),
        };
        self.advance();

        // For numbers
        if let Some(Tok::Number(n)) = self.current() {
            let val = *n;
            self.advance();
            return Ok(Predicate::NumberCmp {
                field: field.to_string(),
                op,
                value: val,
            });
        }

        // For dates or relative dates
        if let Some(Tok::String(s)) | Some(Tok::Ident(s)) = self.current() {
            let s = s.clone();
            self.advance();

            // Check if it's a relative date (e.g., "3d", "2w")
            if let Some((amount, unit)) = parse_relative_duration(&s) {
                return Ok(Predicate::DateCmpRel {
                    field: field.to_string(),
                    op,
                    amount,
                    unit,
                });
            }

            // Otherwise parse as absolute date/datetime now (fail fast)
            let epoch_ms = parse_date_to_epoch_ms(&s)?;
            Ok(Predicate::DateCmpAbs {
                field: field.to_string(),
                op,
                epoch_ms,
            })
        } else {
            Err(MinistoreError::QueryParse("expected value after comparison operator".into()))
        }
    }

    fn current(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) {
        self.pos += 1;
    }

    fn match_tok(&self, expected: &Tok) -> bool {
        matches!(
            (self.current(), expected),
            (Some(Tok::Colon), Tok::Colon)
                | (Some(Tok::And), Tok::And)
                | (Some(Tok::Or), Tok::Or)
                | (Some(Tok::Not), Tok::Not)
                | (Some(Tok::LParen), Tok::LParen)
                | (Some(Tok::RParen), Tok::RParen)
                | (Some(Tok::Gt), Tok::Gt)
                | (Some(Tok::Gte), Tok::Gte)
                | (Some(Tok::Lt), Tok::Lt)
                | (Some(Tok::Lte), Tok::Lte)
                | (Some(Tok::DotDot), Tok::DotDot)
                | (Some(Tok::Ident(_)), Tok::Ident(_))
                | (Some(Tok::String(_)), Tok::String(_))
                | (Some(Tok::Number(_)), Tok::Number(_))
        )
    }

    fn expect(&mut self, tok: &Tok) -> Result<()> {
        if !self.match_tok(tok) {
            return Err(MinistoreError::QueryParse(format!(
                "expected {:?}, got {:?}", tok, self.current()
            )));
        }
        self.advance();
        Ok(())
    }

    fn expect_string_or_ident(&mut self) -> Result<String> {
        match self.current() {
            Some(Tok::String(s)) | Some(Tok::Ident(s)) => {
                let result = s.clone();
                self.advance();
                Ok(result)
            }
            _ => Err(MinistoreError::QueryParse("expected string or identifier".into())),
        }
    }

    fn expect_number(&mut self) -> Result<f64> {
        match self.current() {
            Some(Tok::Number(n)) => {
                let result = *n;
                self.advance();
                Ok(result)
            }
            _ => Err(MinistoreError::QueryParse("expected number".into())),
        }
    }
}

fn classify_keyword_pattern(s: &str) -> KeywordPatternKind {
    // Correct classification:
    // - Exact: no wildcards
    // - Prefix: suffix '*' only (and no other '*' or '?')
    // - Contains: starts and ends with '*' and no other wildcards
    // - Glob: everything else involving '*' or '?'
    if !s.contains('*') && !s.contains('?') {
        return KeywordPatternKind::Exact;
    }
    if s.contains('?') {
        return KeywordPatternKind::Glob;
    }
    // only '*' remains
    let star_count = s.chars().filter(|c| *c == '*').count();
    if s.ends_with('*') && !s.starts_with('*') && star_count == 1 {
        return KeywordPatternKind::Prefix;
    }
    if s.starts_with('*') && s.ends_with('*') && star_count == 2 {
        return KeywordPatternKind::Contains;
    }
    KeywordPatternKind::Glob
}

fn parse_relative_duration(s: &str) -> Option<(i64, RelUnit)> {
    if s.is_empty() {
        return None;
    }

    let (num_part, unit_part) = if let Some(pos) = s.chars().position(|c| c.is_alphabetic()) {
        (&s[..pos], &s[pos..])
    } else {
        return None;
    };

    let amount: i64 = num_part.parse().ok()?;
    
    let unit = match unit_part.to_lowercase().as_str() {
        "h" | "hour" | "hours" => RelUnit::H,
        "d" | "day" | "days" => RelUnit::D,
        "w" | "week" | "weeks" => RelUnit::W,
        "m" | "month" | "months" => RelUnit::M,
        "y" | "year" | "years" => RelUnit::Y,
        _ => return None,
    };

    Some((amount, unit))
}

fn parse_date_to_epoch_ms(s: &str) -> Result<i64> {
    // YYYY-MM-DD => midnight UTC
    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
    }
    // RFC3339
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp_millis());
    }
    // Allow a strict UTC "Z" datetime without offset parsing issues (already covered by RFC3339)
    Err(MinistoreError::QueryParse(format!("invalid date format: {}", s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_has() {
        // "has:title" syntax preferred now for checking field presence
        let expr = parse_query("has:title").unwrap();
        match expr {
            Expr::Pred(Predicate::Has { field }) => assert_eq!(field, "title"),
            _ => panic!("expected Has predicate"),
        }
    }
    
    #[test]
    fn test_parse_bare_text() {
        // "title" is now a text search, not a has check
        let expr = parse_query("title").unwrap();
        match expr {
            Expr::Pred(Predicate::Text { field, fts }) => {
                assert_eq!(field, None);
                assert_eq!(fts, "title");
            }
            _ => panic!("expected Text predicate for bare term"),
        }
    }

    #[test]
    fn test_parse_keyword() {
        let expr = parse_query("tag:rust").unwrap();
        match expr {
            Expr::Pred(Predicate::Keyword { field, pattern, .. }) => {
                assert_eq!(field, "tag");
                assert_eq!(pattern, "rust");
            }
            _ => panic!("expected Keyword predicate"),
        }
    }

    #[test]
    fn test_parse_and_or() {
        let expr = parse_query("a:1 AND b:2 OR c:3").unwrap();
        // Should be: (a:1 AND b:2) OR c:3 due to precedence
        match expr {
            Expr::Or(_, _) => {}
            _ => panic!("expected Or at top level"),
        }
    }

    #[test]
    fn test_parse_not() {
        // "NOT active" is parsed as shorthand "active:false"
        let expr = parse_query("NOT active").unwrap();
        match expr {
            Expr::Pred(Predicate::Bool { field, value }) => {
                assert_eq!(field, "active");
                assert!(!value);
            }
            _ => panic!("expected Bool predicate"),
        }
    }

    #[test]
    fn test_parse_comparison() {
        let expr = parse_query("priority>5").unwrap();
        match expr {
            Expr::Pred(Predicate::NumberCmp { field, op, value }) => {
                assert_eq!(field, "priority");
                assert_eq!(op, CmpOp::Gt);
                assert_eq!(value, 5.0);
            }
            _ => panic!("expected NumberCmp"),
        }
    }

    #[test]
    fn test_parse_range() {
        let expr = parse_query("score:1.0..5.0").unwrap();
        match expr {
            Expr::Pred(Predicate::NumberRange { field, lo, hi }) => {
                assert_eq!(field, "score");
                assert_eq!(lo, 1.0);
                assert_eq!(hi, 5.0);
            }
            _ => panic!("expected NumberRange"),
        }
    }
    
    #[test]
    fn test_parse_not_shorthand() {
        let expr = parse_query("!archived").unwrap();
        match expr {
            Expr::Pred(Predicate::Bool { field, value }) => {
                assert_eq!(field, "archived");
                assert!(!value);
            }
            _ => panic!("expected Bool predicate for shorthand"),
        }
    }
}
