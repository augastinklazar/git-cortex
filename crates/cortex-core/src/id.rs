use crate::error::{CortexError, Result};
use std::fmt;
use std::str::FromStr;
use ulid::Ulid;

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

#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct PrefixedId {
    raw: String,
}

impl PrefixedId {
    pub fn new(prefix: IdPrefix) -> Self {
        let ulid = Ulid::new();
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
        // Must match one of known prefixes
        for prefix in [
            IdPrefix::Tombstone,
            IdPrefix::Handoff,
            IdPrefix::Invariant,
            IdPrefix::Checkpoint,
            IdPrefix::Candidate,
            IdPrefix::Event,
        ] {
            if let Ok(id) = Self::parse(s, prefix) {
                return Ok(id);
            }
        }
        Err(CortexError::InvalidId(
            s.to_string(),
            "known prefix (tmb_, hnd_, inv_, chk_, cand_, evt_)",
        ))
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
}
