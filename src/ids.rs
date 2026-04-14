//! Strongly-typed IDs used across the API.
//!
//! Discord IDs are physically impossible to persist — every user-identifying
//! value in the domain is a [`PseudoId`], a 24-hex-character pseudonym derived
//! as `hex(sha256(discord_id))[0..24]`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Sentinel pseudo_id for mixed/aggregate audio artifacts.
///
/// Accepted in chunk-upload paths where a valid speaker pseudo_id is
/// otherwise required.
pub const MIXED_PSEUDO_ID: &str = "mixed";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PseudoIdError {
    #[error("invalid pseudo_id format: expected 24 hex chars")]
    Invalid,
}

/// A 24-character lowercase hex pseudo_id.
///
/// Validated at construction so the rest of the code can assume the invariant.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct PseudoId(String);

impl PseudoId {
    /// Construct, validating the 24-hex shape.
    pub fn new(raw: impl Into<String>) -> Result<Self, PseudoIdError> {
        let raw = raw.into();
        if is_valid_pseudo_id(&raw) {
            Ok(Self(raw))
        } else {
            Err(PseudoIdError::Invalid)
        }
    }

    /// Accept either a valid pseudo_id or the reserved [`MIXED_PSEUDO_ID`]
    /// sentinel. Used on the chunk-upload path.
    pub fn new_or_mixed(raw: impl Into<String>) -> Result<Self, PseudoIdError> {
        let raw = raw.into();
        if raw == MIXED_PSEUDO_ID || is_valid_pseudo_id(&raw) {
            Ok(Self(raw))
        } else {
            Err(PseudoIdError::Invalid)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_mixed(&self) -> bool {
        self.0 == MIXED_PSEUDO_ID
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl fmt::Display for PseudoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for PseudoId {
    type Err = PseudoIdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl<'de> Deserialize<'de> for PseudoId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        PseudoId::new_or_mixed(raw).map_err(serde::de::Error::custom)
    }
}

impl sqlx::Type<sqlx::Postgres> for PseudoId {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for PseudoId {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(PseudoId(s))
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for PseudoId {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        <String as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.0, buf)
    }
}

/// Return true iff the string matches `^[0-9a-f]{24}$`.
fn is_valid_pseudo_id(s: &str) -> bool {
    s.len() == 24 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A caller-supplied deduplication key.
///
/// Used by chunk uploads (`X-Client-Chunk-Id`) and bulk CRUD inserts
/// (`client_id` in the JSON body). Opaque to the server beyond length checks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClientId(pub String);

impl ClientId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An opaque revision marker for optimistic concurrency.
///
/// Generated server-side, sent as `ETag`, echoed back in `If-Match`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ETag(pub String);

impl ETag {
    /// Generate a fresh random 16-hex-character tag.
    pub fn new_random() -> Self {
        use rand::RngCore;
        let mut buf = [0u8; 8];
        rand::rng().fill_bytes(&mut buf);
        Self(hex::encode(buf))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ETag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl sqlx::Type<sqlx::Postgres> for ETag {
    fn type_info() -> sqlx::postgres::PgTypeInfo {
        <String as sqlx::Type<sqlx::Postgres>>::type_info()
    }
}

impl<'r> sqlx::Decode<'r, sqlx::Postgres> for ETag {
    fn decode(
        value: sqlx::postgres::PgValueRef<'r>,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let s = <String as sqlx::Decode<sqlx::Postgres>>::decode(value)?;
        Ok(ETag(s))
    }
}

impl<'q> sqlx::Encode<'q, sqlx::Postgres> for ETag {
    fn encode_by_ref(
        &self,
        buf: &mut sqlx::postgres::PgArgumentBuffer,
    ) -> Result<sqlx::encode::IsNull, Box<dyn std::error::Error + Send + Sync>> {
        <String as sqlx::Encode<sqlx::Postgres>>::encode_by_ref(&self.0, buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pseudo_id_valid_24_hex() {
        assert!(PseudoId::new("0123456789abcdef01234567").is_ok());
        assert!(PseudoId::new("ffffffffffffffffffffffff").is_ok());
    }

    #[test]
    fn pseudo_id_rejects_too_short() {
        assert_eq!(PseudoId::new("abc").unwrap_err(), PseudoIdError::Invalid);
        assert_eq!(
            PseudoId::new("0123456789abcdef").unwrap_err(),
            PseudoIdError::Invalid
        );
    }

    #[test]
    fn pseudo_id_rejects_too_long() {
        assert_eq!(
            PseudoId::new("0123456789abcdef0123456789").unwrap_err(),
            PseudoIdError::Invalid
        );
    }

    #[test]
    fn pseudo_id_rejects_non_hex() {
        assert_eq!(
            PseudoId::new("GGGGGGGGGGGGGGGGGGGGGGGG").unwrap_err(),
            PseudoIdError::Invalid
        );
        assert_eq!(
            PseudoId::new("0123456789ABCDEF01234567").unwrap_err(),
            PseudoIdError::Invalid
        );
    }

    #[test]
    fn pseudo_id_mixed_sentinel() {
        assert!(PseudoId::new_or_mixed("mixed").is_ok());
        assert!(PseudoId::new_or_mixed("mixed").unwrap().is_mixed());
        assert!(!PseudoId::new("0123456789abcdef01234567")
            .unwrap()
            .is_mixed());
    }
}
