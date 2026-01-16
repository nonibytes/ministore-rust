use crate::index::RankMode;
use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Sha256, Digest};
use rand::{RngCore, rngs::OsRng};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum CursorPayload {
    // ORDER BY score DESC, item_id ASC
    Fts { score: f64, item_id: i64 },
    // ORDER BY updated_at DESC, path ASC
    Recency { updated_at_ms: i64, path: String },
    // ORDER BY rank_value DESC, updated_at DESC, path ASC
    Field { field: String, rank_value: f64, updated_at_ms: i64, path: String },
    // ORDER BY item_id ASC
    None { item_id: i64 },
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

pub fn is_short_cursor_token(token: &str) -> bool {
    token.starts_with("c:")
}

pub fn short_cursor_handle(token: &str) -> Option<&str> {
    token.strip_prefix("c:")
}

pub fn make_short_handle() -> String {
    // base62-ish: we'll hex-encode 6 random bytes for simplicity and stability
    let mut b = [0u8; 6];
    OsRng.fill_bytes(&mut b);
    format!("{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}", b[0], b[1], b[2], b[3], b[4], b[5])
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
        let payload = CursorPayload::Recency { updated_at_ms: 123, path: "/p".into() };
        
        let hash = "abc123";
        let encoded = encode_full(&payload, hash).unwrap();
        let decoded = decode_full(&encoded).unwrap();
        
        assert_eq!(decoded.payload, payload);
        assert_eq!(decoded.hash, hash);
    }

    #[test]
    fn test_short_handle() {
        let h = make_short_handle();
        assert!(!h.is_empty());
    }
}
