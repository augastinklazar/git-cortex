use crate::error::{CortexError, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::str::FromStr;
use std::sync::{LazyLock, Mutex};
use ulid::{Generator, Ulid};

static GENERATOR: LazyLock<Mutex<Generator>> = LazyLock::new(|| Mutex::new(Generator::new()));

fn is_valid_crockford_base32_char(c: char) -> bool {
    matches!(c, '0'..='9' | 'A'..='H' | 'J'..='K' | 'M'..='N' | 'P'..='T' | 'V'..='Z')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdPrefix {
    Tombstone,
    Handoff,
    Invariant,
    Checkpoint,
    Candidate,
    Event,
}

impl IdPrefix {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Tombstone => "tmb_",
            Self::Handoff => "hnd_",
            Self::Invariant => "inv_",
            Self::Checkpoint => "chk_",
            Self::Candidate => "cand_",
            Self::Event => "evt_",
        }
    }
}

impl fmt::Display for IdPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrefixedId {
    raw: String,
}

impl PrefixedId {
    pub fn new(prefix: IdPrefix) -> Self {
        let ulid = GENERATOR
            .lock()
            .map(|mut gen| gen.generate().unwrap_or_else(|_| Ulid::new()))
            .unwrap_or_else(|_| Ulid::new());
        Self {
            raw: format!("{}{}", prefix.as_str(), ulid),
        }
    }

    pub fn parse(s: &str, expected_prefix: IdPrefix) -> Result<Self> {
        let p = expected_prefix.as_str();
        if !s.starts_with(p) {
            return Err(CortexError::InvalidId(s.to_string(), p));
        }
        let ulid_part = &s[p.len()..];
        if ulid_part.len() != 26 || !ulid_part.chars().all(is_valid_crockford_base32_char) {
            return Err(CortexError::InvalidId(s.to_string(), p));
        }
        Ulid::from_string(ulid_part).map_err(|_| CortexError::InvalidId(s.to_string(), p))?;
        Ok(Self { raw: s.to_string() })
    }

    pub fn as_str(&self) -> &str {
        &self.raw
    }

    pub fn prefix(&self) -> Option<IdPrefix> {
        if self.raw.starts_with(IdPrefix::Tombstone.as_str()) {
            Some(IdPrefix::Tombstone)
        } else if self.raw.starts_with(IdPrefix::Handoff.as_str()) {
            Some(IdPrefix::Handoff)
        } else if self.raw.starts_with(IdPrefix::Invariant.as_str()) {
            Some(IdPrefix::Invariant)
        } else if self.raw.starts_with(IdPrefix::Checkpoint.as_str()) {
            Some(IdPrefix::Checkpoint)
        } else if self.raw.starts_with(IdPrefix::Candidate.as_str()) {
            Some(IdPrefix::Candidate)
        } else if self.raw.starts_with(IdPrefix::Event.as_str()) {
            Some(IdPrefix::Event)
        } else {
            None
        }
    }
}

impl fmt::Display for PrefixedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl FromStr for PrefixedId {
    type Err = CortexError;

    fn from_str(s: &str) -> Result<Self> {
        for prefix in [
            IdPrefix::Tombstone,
            IdPrefix::Handoff,
            IdPrefix::Invariant,
            IdPrefix::Checkpoint,
            IdPrefix::Candidate,
            IdPrefix::Event,
        ] {
            if s.starts_with(prefix.as_str()) {
                return Self::parse(s, prefix);
            }
        }
        Err(CortexError::InvalidId(
            s.to_string(),
            "known prefix (tmb_, hnd_, inv_, chk_, cand_, evt_)",
        ))
    }
}

impl Serialize for PrefixedId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for PrefixedId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse::<PrefixedId>().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_id_generation_and_parsing() {
        let tmb_id = PrefixedId::new(IdPrefix::Tombstone);
        assert!(tmb_id.as_str().starts_with("tmb_"));
        assert_eq!(tmb_id.prefix(), Some(IdPrefix::Tombstone));

        let parsed = PrefixedId::parse(tmb_id.as_str(), IdPrefix::Tombstone).unwrap();
        assert_eq!(parsed, tmb_id);

        let err = PrefixedId::parse(tmb_id.as_str(), IdPrefix::Invariant);
        assert!(err.is_err());
    }

    #[test]
    fn test_monotonic_generation() {
        let mut ids = Vec::with_capacity(100);
        for _ in 0..100 {
            ids.push(PrefixedId::new(IdPrefix::Candidate));
        }
        for window in ids.windows(2) {
            assert!(
                window[0].as_str() < window[1].as_str(),
                "IDs must be strictly monotonic: {} vs {}",
                window[0],
                window[1]
            );
        }
    }

    #[test]
    fn test_crockford_base32_case_strict_rejection() {
        let valid = PrefixedId::new(IdPrefix::Tombstone);
        let raw = valid.as_str();

        // Lowercase variant must be rejected
        let lowercase = raw.to_lowercase();
        assert!(PrefixedId::parse(&lowercase, IdPrefix::Tombstone).is_err());
        assert!(lowercase.parse::<PrefixedId>().is_err());

        // Non-Crockford characters ('I', 'L', 'O', 'U') must be rejected
        let mut chars: Vec<char> = raw.chars().collect();
        chars[10] = 'I';
        let with_i: String = chars.iter().collect();
        assert!(PrefixedId::parse(&with_i, IdPrefix::Tombstone).is_err());

        chars[10] = 'O';
        let with_o: String = chars.iter().collect();
        assert!(PrefixedId::parse(&with_o, IdPrefix::Tombstone).is_err());

        chars[10] = 'U';
        let with_u: String = chars.iter().collect();
        assert!(PrefixedId::parse(&with_u, IdPrefix::Tombstone).is_err());

        chars[10] = 'L';
        let with_l: String = chars.iter().collect();
        assert!(PrefixedId::parse(&with_l, IdPrefix::Tombstone).is_err());

        // Invalid length must be rejected
        let truncated = &raw[..raw.len() - 1];
        assert!(PrefixedId::parse(truncated, IdPrefix::Tombstone).is_err());
    }

    #[test]
    fn test_serde_json_roundtrip_and_strictness() {
        let id = PrefixedId::new(IdPrefix::Invariant);
        let serialized = serde_json::to_string(&id).unwrap();
        let deserialized: PrefixedId = serde_json::from_str(&serialized).unwrap();
        assert_eq!(id, deserialized);

        // Lowercase in JSON must fail deserialization
        let lowercase_json = format!("\"{}\"", id.as_str().to_lowercase());
        let res: std::result::Result<PrefixedId, _> = serde_json::from_str(&lowercase_json);
        assert!(res.is_err());
    }
}
