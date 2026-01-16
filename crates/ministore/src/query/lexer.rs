use crate::{MinistoreError, Result};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    // Literals
    Ident(String),
    String(String),
    Number(f64),

    // Operators
    Colon,      // :
    And,        // AND or &&
    Or,         // OR or ||
    Not,        // NOT or !
    LParen,     // (
    RParen,     // )
    Gt,         // >
    Gte,        // >=
    Lt,         // <
    Lte,        // <=
    DotDot,     // ..
}

/// Tokenize a query string.
pub fn lex(input: &str) -> Result<Vec<Tok>> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        // Skip whitespace
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }

        // Two-char operators
        if i + 1 < chars.len() {
            let two_char = format!("{}{}", chars[i], chars[i + 1]);
            match two_char.as_str() {
                ">=" => {
                    tokens.push(Tok::Gte);
                    i += 2;
                    continue;
                }
                "<=" => {
                    tokens.push(Tok::Lte);
                    i += 2;
                    continue;
                }
                ".." => {
                    tokens.push(Tok::DotDot);
                    i += 2;
                    continue;
                }
                "&&" => {
                    tokens.push(Tok::And);
                    i += 2;
                    continue;
                }
                "||" => {
                    tokens.push(Tok::Or);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }

        // Single-char operators
        match chars[i] {
            ':' => {
                tokens.push(Tok::Colon);
                i += 1;
                continue;
            }
            '>' => {
                tokens.push(Tok::Gt);
                i += 1;
                continue;
            }
            '<' => {
                tokens.push(Tok::Lt);
                i += 1;
                continue;
            }
            '!' => {
                tokens.push(Tok::Not);
                i += 1;
                continue;
            }
            '(' => {
                tokens.push(Tok::LParen);
                i += 1;
                continue;
            }
            ')' => {
                tokens.push(Tok::RParen);
                i += 1;
                continue;
            }
            _ => {}
        }

        // String literal (quoted)
        if chars[i] == '"' {
            i += 1; // skip opening quote
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 2; // skip escape sequence
                } else {
                    i += 1;
                }
            }
            if i >= chars.len() {
                return Err(MinistoreError::QueryParse("unterminated string literal".into()));
            }
            let s: String = chars[start..i].iter().collect();
            tokens.push(Tok::String(unescape(&s)?));
            i += 1; // skip closing quote
            continue;
        }

        // Number
        if chars[i].is_ascii_digit() || (chars[i] == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
            let start = i;
            if chars[i] == '-' {
                i += 1;
            }
            let mut has_dot = false;
            while i < chars.len() {
                if chars[i].is_ascii_digit() {
                    i += 1;
                } else if chars[i] == '.' && !has_dot {
                    // Check if this is .. (range operator)
                    if i + 1 < chars.len() && chars[i + 1] == '.' {
                        break; // Stop before ..
                    }
                    has_dot = true;
                    i += 1;
                } else {
                    break;
                }
            }
            let num_str: String = chars[start..i].iter().collect();
            let num = num_str.parse::<f64>()
                .map_err(|_| MinistoreError::QueryParse(format!("invalid number: {}", num_str)))?;
            tokens.push(Tok::Number(num));
            continue;
        }

        // Identifier or keyword
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '*' || chars[i] == '?') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect();
            
            // Check for keywords (case-insensitive)
            match ident.to_uppercase().as_str() {
                "AND" => tokens.push(Tok::And),
                "OR" => tokens.push(Tok::Or),
                "NOT" => tokens.push(Tok::Not),
                _ => tokens.push(Tok::Ident(ident)),
            }
            continue;
        }

        // If we get here, unrecognized character
        return Err(MinistoreError::QueryParse(format!("unexpected character: {}", chars[i])));
    }

    Ok(tokens)
}

fn unescape(s: &str) -> Result<String> {
    let mut result = String::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                'n' => result.push('\n'),
                't' => result.push('\t'),
                'r' => result.push('\r'),
                '"' => result.push('"'),
                '\\' => result.push('\\'),
                _ => return Err(MinistoreError::QueryParse(format!("invalid escape sequence: \\{}", chars[i + 1]))),
            }
            i += 2;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lex_simple() {
        let tokens = lex("title:test").unwrap();
        assert_eq!(tokens.len(), 3);
        assert_eq!(tokens[0], Tok::Ident("title".to_string()));
        assert_eq!(tokens[1], Tok::Colon);
        assert_eq!(tokens[2], Tok::Ident("test".to_string()));
    }

    #[test]
    fn test_lex_operators() {
        let tokens = lex("priority>5 AND active").unwrap();
        assert_eq!(tokens[1], Tok::Gt);
        assert_eq!(tokens[3], Tok::And);
    }

    #[test]
    fn test_lex_string() {
        let tokens = lex(r#"title:"hello world""#).unwrap();
        assert_eq!(tokens[2], Tok::String("hello world".to_string()));
    }

    #[test]
    fn test_lex_number() {
        let tokens = lex("3.14").unwrap();
        assert_eq!(tokens[0], Tok::Number(3.14));
    }

    #[test]
    fn test_lex_parens() {
        let tokens = lex("(a OR b) AND c").unwrap();
        assert_eq!(tokens[0], Tok::LParen);
        assert_eq!(tokens[4], Tok::RParen);
    }
}
