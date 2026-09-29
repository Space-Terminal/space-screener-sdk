//! The release document the registry signs when a moderator approves a version.
//!
//! The registry signs the exact bytes of the compact JSON below (fields in declaration order)
//! and serves them as a string next to the signature. Verify the signature over those bytes
//! as received — never over a re-serialization — then parse them with [`Release::parse`].

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub v: u32,
    pub id: String,
    pub version: String,
    pub abi: u32,
    pub min_terminal: String,
    /// Lower-case hex.
    pub wasm_sha256: String,
    pub wasm_size: u64,
    /// Lower-case hex.
    pub manifest_sha256: String,
    /// RFC 3339.
    pub approved_at: String,
}

impl Release {
    /// The bytes to sign.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn parse(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_bytes_keep_the_documented_field_order() {
        let release = Release {
            v: VERSION,
            id: "ivan.oi".to_string(),
            version: "1.2.0".to_string(),
            abi: 1,
            min_terminal: "0.104.70".to_string(),
            wasm_sha256: "ab".to_string(),
            wasm_size: 42,
            manifest_sha256: "cd".to_string(),
            approved_at: "2026-09-29T12:00:00Z".to_string(),
        };
        let json = release.to_json().unwrap();
        assert_eq!(
            json,
            r#"{"v":1,"id":"ivan.oi","version":"1.2.0","abi":1,"min_terminal":"0.104.70","wasm_sha256":"ab","wasm_size":42,"manifest_sha256":"cd","approved_at":"2026-09-29T12:00:00Z"}"#
        );
        assert_eq!(Release::parse(&json).unwrap(), release);
    }
}
