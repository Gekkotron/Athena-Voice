use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum IdError {
    #[error("invalid SatelliteId `{0}`: must match ^[a-z0-9-]{{1,64}}$")]
    InvalidSatelliteId(String),
    #[error("invalid Locale `{0}`: must match ^[a-z]{{2}}(-[A-Z]{{2}})?$")]
    InvalidLocale(String),
    #[error("invalid SessionId: {0}")]
    InvalidSessionId(#[from] uuid::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub Uuid);

impl SessionId {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for SessionId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SatelliteId(String);

impl SatelliteId {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        if s.is_empty()
            || s.len() > 64
            || !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err(IdError::InvalidSatelliteId(s));
        }
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SatelliteId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for SatelliteId {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<SatelliteId> for String {
    fn from(v: SatelliteId) -> Self {
        v.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Locale(String);

impl Locale {
    pub fn new(s: impl Into<String>) -> Result<Self, IdError> {
        let s = s.into();
        let bytes = s.as_bytes();
        let ok = match bytes.len() {
            2 => bytes.iter().all(u8::is_ascii_lowercase),
            5 => {
                bytes[0].is_ascii_lowercase()
                    && bytes[1].is_ascii_lowercase()
                    && bytes[2] == b'-'
                    && bytes[3].is_ascii_uppercase()
                    && bytes[4].is_ascii_uppercase()
            }
            _ => false,
        };
        if !ok {
            return Err(IdError::InvalidLocale(s));
        }
        Ok(Self(s))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Locale {
    type Error = IdError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl From<Locale> for String {
    fn from(v: Locale) -> Self {
        v.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_new_v4_is_unique() {
        let a = SessionId::new_v4();
        let b = SessionId::new_v4();
        assert_ne!(a, b);
    }

    #[test]
    fn session_id_roundtrip_via_display_fromstr() {
        let a = SessionId::new_v4();
        let s = a.to_string();
        let b: SessionId = s.parse().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let a = SessionId::new_v4();
        let json = serde_json::to_string(&a).unwrap();
        let b: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn satellite_id_accepts_valid() {
        assert!(SatelliteId::new("phone-01").is_ok());
        assert!(SatelliteId::new("a").is_ok());
        assert!(SatelliteId::new("phone-abc-123").is_ok());
    }

    #[test]
    fn satellite_id_rejects_invalid() {
        assert!(SatelliteId::new("").is_err());
        assert!(SatelliteId::new("Phone-01").is_err());
        assert!(SatelliteId::new("phone_01").is_err());
        assert!(SatelliteId::new("phone/01").is_err());
        assert!(SatelliteId::new(&"a".repeat(65)).is_err());
    }

    #[test]
    fn locale_accepts_valid() {
        assert!(Locale::new("fr").is_ok());
        assert!(Locale::new("en").is_ok());
        assert!(Locale::new("fr-FR").is_ok());
        assert!(Locale::new("en-US").is_ok());
    }

    #[test]
    fn locale_rejects_invalid() {
        assert!(Locale::new("").is_err());
        assert!(Locale::new("FR").is_err());
        assert!(Locale::new("french").is_err());
        assert!(Locale::new("fr-fr").is_err());
        assert!(Locale::new("fr_FR").is_err());
    }
}
