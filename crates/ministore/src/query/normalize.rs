use crate::query::ast::*;
use crate::{MinistoreError, Result};
use crate::constants::{MIN_CONTAINS_LEN, MIN_PREFIX_LEN};

/// Normalize and validate query expression.
/// Checks for positive anchor and validates keyword patterns.
pub fn normalize(expr: Expr) -> Result<Expr> {
    // Check for positive anchor
    if !has_positive_anchor(&expr) {
        return Err(MinistoreError::QueryRejected(
            "query must have at least one positive anchor (FTS term, exact keyword match, or literal path prefix)".into()
        ));
    }
    
    // Validate keyword patterns
    validate_patterns(&expr)?;
    
    Ok(expr)
}

/// Check if expression has a positive anchor.
/// A positive anchor is: FTS term, exact keyword match (no wildcards), or path with literal prefix.
pub fn has_positive_anchor(expr: &Expr) -> bool {
    match expr {
        Expr::And(left, right) => {
            has_positive_anchor(left) || has_positive_anchor(right)
        }
        Expr::Or(left, right) => {
            has_positive_anchor(left) || has_positive_anchor(right)
        }
        Expr::Not(_) => false, // Negation is not a positive anchor
        Expr::Pred(pred) => is_positive_anchor(pred),
    }
}

fn is_positive_anchor(pred: &Predicate) -> bool {
    match pred {
        Predicate::Text { .. } => true, // FTS queries are positive anchors
        Predicate::Keyword { kind, pattern, .. } => {
            match kind {
                KeywordPatternKind::Exact => true, // Exact matches are positive
                KeywordPatternKind::Prefix => has_literal_prefix(pattern), // Wildcards with literal prefix are positive
                KeywordPatternKind::Contains => has_literal_prefix(pattern), // Contains with literal prefix is positive
                KeywordPatternKind::Glob => has_literal_prefix(pattern), // Glob with literal prefix is positive
            }
        }
        Predicate::PathGlob { pattern } => {
            // Path is positive if it has a literal prefix
            has_literal_prefix(pattern)
        }
        // Date and number predicates are positive anchors (they target indexed fields)
        Predicate::DateCmpAbs { .. } => true,
        Predicate::DateRangeAbs { .. } => true,
        Predicate::DateCmpRel { .. } => true,
        Predicate::NumberCmp { .. } => true,
        Predicate::NumberRange { .. } => true,
        _ => false,
    }
}

fn has_literal_prefix(pattern: &str) -> bool {
    // Check if pattern starts with literal characters before any wildcard
    if let Some(pos) = pattern.find(&['*', '?'][..]) {
        pos > 0 // Has literal chars before wildcard
    } else {
        true // No wildcards at all
    }
}

/// Validate keyword patterns against guardrails.
fn validate_patterns(expr: &Expr) -> Result<()> {
    match expr {
        Expr::And(left, right) | Expr::Or(left, right) => {
            validate_patterns(left)?;
            validate_patterns(right)?;
        }
        Expr::Not(inner) => {
            validate_patterns(inner)?;
        }
        Expr::Pred(pred) => {
            validate_predicate_pattern(pred)?;
        }
    }
    Ok(())
}

fn validate_predicate_pattern(pred: &Predicate) -> Result<()> {
    match pred {
        Predicate::Keyword { pattern, kind, .. } => {
            match kind {
                KeywordPatternKind::Prefix => {
                    let prefix = pattern.trim_end_matches('*');
                    if prefix.len() < MIN_PREFIX_LEN {
                        return Err(MinistoreError::QueryRejected(
                            format!("prefix pattern '{}*' too short (minimum {} characters)", prefix, MIN_PREFIX_LEN)
                        ));
                    }
                }
                KeywordPatternKind::Contains => {
                    // Extract inner pattern between wildcards
                    let inner = pattern.trim_matches('*');
                    if inner.len() < MIN_CONTAINS_LEN {
                        return Err(MinistoreError::QueryRejected(
                            format!("contains pattern '*{}*' too short (minimum {} characters)", inner, MIN_CONTAINS_LEN)
                        ));
                    }
                }
                KeywordPatternKind::Glob => {
                    // Require a literal prefix before any wildcard (performance guardrail).
                    if pattern.contains(&['*', '?'][..]) {
                        if let Some(pos) = pattern.find(&['*', '?'][..]) {
                            let prefix = &pattern[..pos];
                            if prefix.is_empty() {
                                return Err(MinistoreError::QueryRejected(
                                    "glob pattern must have a literal prefix before wildcards".into()
                                ));
                            }
                            if prefix.len() < MIN_PREFIX_LEN {
                                return Err(MinistoreError::QueryRejected(
                                    format!("glob prefix '{}' too short (minimum {} characters)", prefix, MIN_PREFIX_LEN)
                                ));
                            }
                        }
                    }
                }
                _ => {} // Exact doesn't need validation
            }
        }
        Predicate::PathGlob { pattern } => {
            // Validate path has literal prefix before wildcards
            if pattern.contains(&['*', '?'][..]) && !has_literal_prefix(pattern) {
                return Err(MinistoreError::QueryRejected(
                    "path pattern must have literal prefix before wildcards".into()
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::parser::parse_query;

    #[test]
    fn test_positive_anchor_simple() {
        let expr = parse_query("title:test").unwrap();
        assert!(has_positive_anchor(&expr));
    }

    #[test]
    fn test_positive_anchor_with_not() {
        let expr = parse_query("title:test AND NOT archived").unwrap();
        assert!(has_positive_anchor(&expr));
    }

    #[test]
    fn test_no_positive_anchor() {
        let expr = parse_query("NOT archived").unwrap();
        assert!(!has_positive_anchor(&expr));
    }

    #[test]
    fn test_normalize_rejects_pure_negative() {
        let expr = parse_query("NOT archived").unwrap();
        assert!(normalize(expr).is_err());
    }

    #[test]
    fn test_normalize_accepts_positive() {
        let expr = parse_query("title:test").unwrap();
        assert!(normalize(expr).is_ok());
    }
}
