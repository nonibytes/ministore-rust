use crate::index::RankMode;
use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Sha256, Digest};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CursorPayload {
    pub item_id: i64,
    pub rank_value: Option<f64>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorPosition {
    pub payload: CursorPayload,
    pub hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RankModeSer {
    Default,
    Recency,
    Field(String),
    None,
}

impl From<&RankMode> for RankModeSer {
    fn from(mode: &RankMode) -> Self {
        match mode {
            RankMode::Default => RankModeSer::Default,
            RankMode::Recency => RankModeSer::Recency,
            RankMode::Field(f) => RankModeSer::Field(f.clone()),
            RankMode::None => RankModeSer::None,
        }
    }
}

/// Generate a hash for cursor validation.
/// Combines schema_json, query, and rank mode to detect schema/query changes.
pub fn hash_query(schema_json: &str, query: &str, rank: &RankModeSer) -> String {
    let mut hasher = Sha256::new();
    // Use separators to avoid accidental concatenation ambiguity
    hasher.update(schema_json.as_bytes());
    hasher.update(b"\n");
    hasher.update(query.as_bytes());
    hasher.update(b"\n");
    hasher.update(serde_json::to_string(rank).unwrap_or_default().as_bytes());
    
    let result = hasher.finalize();
    format!("{:x}", result) // full hex for collision resistance
}

/// Encode cursor payload to base64 string (full mode).
pub fn encode_full(payload: &CursorPayload, hash: &str) -> crate::Result<String> {
    let pos = CursorPosition {
        payload: payload.clone(),
        hash: hash.to_string(),
    };
    
    let json = serde_json::to_string(&pos)?;
    Ok(URL_SAFE_NO_PAD.encode(json.as_bytes()))
}

/// Decode cursor from base64 string (full mode).
pub fn decode_full(token: &str) -> crate::Result<CursorPosition> {
    let bytes = URL_SAFE_NO_PAD.decode(token.as_bytes())
        .map_err(|e| crate::MinistoreError::Cursor(format!("base64 decode error: {}", e)))?;
    
    let json = String::from_utf8(bytes)
        .map_err(|e| crate::MinistoreError::Cursor(format!("utf8 error: {}", e)))?;
    
    let pos: CursorPosition = serde_json::from_str(&json)
        .map_err(|e| crate::MinistoreError::Cursor(format!("json parse error: {}", e)))?;
    
    Ok(pos)
}

/// Encode cursor as short format (just search row position).
pub fn encode_short(item_id: i64) -> String {
    format!("s{}", item_id)
}

/// Decode short cursor.
pub fn decode_short(token: &str) -> crate::Result<i64> {
    if !token.starts_with('s') {
        return Err(crate::MinistoreError::Cursor(
            "short cursor must start with 's'".into()
        ));
    }
    
    token[1..].parse::<i64>()
        .map_err(|e| crate::MinistoreError::Cursor(format!("invalid item_id: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_query() {
        let hash1 = hash_query("schema1", "query1", &RankModeSer::Default);
        let hash2 = hash_query("schema1", "query1", &RankModeSer::Default);
        let hash3 = hash_query("schema2", "query1", &RankModeSer::Default);
        
        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_encode_decode_full() {
        let payload = CursorPayload {
            item_id: 123,
            rank_value: Some(0.95),
            path: "/test/path".to_string(),
        };
        
        let hash = "abc123";
        let encoded = encode_full(&payload, hash).unwrap();
        let decoded = decode_full(&encoded).unwrap();
        
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.hash, hash);
    }

    #[test]
    fn test_encode_decode_short() {
        let item_id = 456;
        let encoded = encode_short(item_id);
        let decoded = decode_short(&encoded).unwrap();
        
        assert_eq!(decoded, item_id);
        assert!(encoded.starts_with('s'));
    }

    #[test]
    fn test_decode_short_invalid() {
        assert!(decode_short("x123").is_err());
        assert!(decode_short("s").is_err());
        assert!(decode_short("sabc").is_err());
    }
}
