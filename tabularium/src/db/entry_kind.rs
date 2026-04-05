//! [`EntryKind`] — directory vs file; SQLite stores fixed integer discriminants with `sqlx` typing.

use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteArgumentValue, SqliteTypeInfo, SqliteValueRef};
use sqlx::{Decode, Encode, Sqlite, Type};

/// Stored in SQLite as `0` = directory, `1` = file.
#[repr(i64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum EntryKind {
    Dir = 0,
    File = 1,
}

impl TryFrom<i64> for EntryKind {
    type Error = BoxDynError;

    fn try_from(v: i64) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(Self::Dir),
            1 => Ok(Self::File),
            _ => Err(format!("unknown entry kind discriminant {v}").into()),
        }
    }
}

impl Type<Sqlite> for EntryKind {
    fn type_info() -> SqliteTypeInfo {
        <i64 as Type<Sqlite>>::type_info()
    }
}

impl<'q> Encode<'q, Sqlite> for EntryKind {
    fn encode_by_ref(&self, buf: &mut Vec<SqliteArgumentValue<'q>>) -> Result<IsNull, BoxDynError> {
        Encode::<Sqlite>::encode_by_ref(&(*self as i64), buf)
    }
}

impl<'r> Decode<'r, Sqlite> for EntryKind {
    fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
        let v = <i64 as Decode<Sqlite>>::decode(value)?;
        Self::try_from(v)
    }
}
