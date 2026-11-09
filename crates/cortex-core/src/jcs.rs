use crate::error::{CortexError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// Serialize a serde value into canonical JSON (RFC 8785)
pub fn to_canonical_json<T: Serialize>(value: &T) -> Result<String> {
    serde_jcs::to_string(value)
        .map_err(|e| CortexError::Canonicalization(e.to_string()))
}

/// Compute SHA-256 digest over the RFC 8785 canonical JSON serialization
pub fn canonical_hash<T: Serialize>(value: &T) -> Result<String> {
    let json = to_canonical_json(value)?;
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let digest = hasher.finalize();
    Ok(format!("sha256:{}", hex::encode(digest)))
}

/// Compute SHA-256 digest over arbitrary raw bytes
pub fn sha256_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    format!("sha256:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_canonical_json_key_order() {
        let val1 = json!({ "b": 1, "a": 2 });
        let val2 = json!({ "a": 2, "b": 1 });

        let str1 = to_canonical_json(&val1).unwrap();
        let str2 = to_canonical_json(&val2).unwrap();

        assert_eq!(str1, str2);
        assert_eq!(str1, r#"{"a":2,"b":1}"#);

        let hash1 = canonical_hash(&val1).unwrap();
        let hash2 = canonical_hash(&val2).unwrap();
        assert_eq!(hash1, hash2);
    }
}
