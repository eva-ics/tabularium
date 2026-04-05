//! Crate-wide errors — the Emperor permits no silent corruption.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Library result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// All failure modes surfaced to callers.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("duplicate: {0}")]
    Duplicate(String),
    #[error("not empty: {0}")]
    NotEmpty(String),
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("sql: {0}")]
    Sql(String),
    #[error("search index: {0}")]
    Search(String),
    #[error("io: {0}")]
    Io(String),
}

impl Serialize for Error {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Error {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(Error::InvalidInput(s))
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::Io(value.to_string())
    }
}

#[cfg(feature = "db")]
impl From<sqlx::Error> for Error {
    fn from(value: sqlx::Error) -> Self {
        if let Some(db_err) = value.as_database_error()
            && db_err.is_unique_violation()
        {
            return Error::Duplicate(db_err.message().to_string());
        }
        Error::Sql(value.to_string())
    }
}

#[cfg(feature = "db")]
impl From<tantivy::TantivyError> for Error {
    fn from(value: tantivy::TantivyError) -> Self {
        Error::Search(value.to_string())
    }
}
