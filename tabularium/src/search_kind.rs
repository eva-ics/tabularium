//! Search engine selection shared by every transport.

use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// Lexical (Tantivy), semantic (embeddings), or their combined ranking.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    Lexical,
    Semantic,
    #[default]
    Hybrid,
}
impl SearchKind {
    /// Canonical CLI/config/wire spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lexical => "lexical",
            Self::Semantic => "semantic",
            Self::Hybrid => "hybrid",
        }
    }
}
impl fmt::Display for SearchKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl FromStr for SearchKind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        match s {
            "lexical" => Ok(Self::Lexical),
            "semantic" => Ok(Self::Semantic),
            "hybrid" => Ok(Self::Hybrid),
            _ => Err(Error::InvalidInput(
                "kind must be lexical, semantic, or hybrid".into(),
            )),
        }
    }
}
