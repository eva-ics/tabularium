//! Additive search ordering and modification-time filters.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    #[default]
    Relevance,
    ModifiedDesc,
}
impl fmt::Display for SearchSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Relevance => "relevance",
            Self::ModifiedDesc => "modified_desc",
        })
    }
}
impl FromStr for SearchSort {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "relevance" => Ok(Self::Relevance),
            "modified_desc" => Ok(Self::ModifiedDesc),
            _ => Err(Error::InvalidInput(
                "sort must be relevance or modified_desc".into(),
            )),
        }
    }
}

/// Bounds are exclusive UNIX nanoseconds, matching document modified_at.
/// Modification time is not an assertion of the content's truth or currency.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SearchOptions {
    #[serde(default)]
    pub sort: SearchSort,
    pub modified_after: Option<u64>,
    pub modified_before: Option<u64>,
}
impl SearchOptions {
    pub fn validate(&self) -> Result<()> {
        if let (Some(after), Some(before)) = (self.modified_after, self.modified_before)
            && after > before
        {
            return Err(Error::InvalidInput(
                "modified_after must not exceed modified_before".into(),
            ));
        }
        Ok(())
    }
    pub fn matches_modified(&self, nanos: u128) -> bool {
        self.modified_after.is_none_or(|v| nanos > u128::from(v))
            && self.modified_before.is_none_or(|v| nanos < u128::from(v))
    }
}
