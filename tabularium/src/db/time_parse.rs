//! User-facing timestamp parsing for CLI and other callers (`db` feature).

use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};

use crate::{Error, Result, Timestamp};

/// Parse a CLI time string: unsigned integer → Unix **seconds**; else strict `YYYY-MM-DD[ T]HH:MM:SS`
/// as UTC (before `dateparser`, so the same string is not timezone-ambiguous); then `dateparser`,
/// RFC3339, naive UTC again for other shapes.
pub fn parse_user_timestamp(s: &str) -> Result<Timestamp> {
    let s = s.trim();
    if s.is_empty() {
        return Err(Error::InvalidInput("time must not be empty".into()));
    }

    if let Ok(secs) = s.parse::<u64>() {
        return Ok(Timestamp::from_secs(secs));
    }

    if s.len() >= 19
        && matches!(s.as_bytes().get(10), Some(b' ' | b'T'))
        && let Some(ts) = try_naive_utc(s)
    {
        return Ok(ts);
    }

    try_dateparser(s)
        .or_else(|| try_rfc3339(s))
        .or_else(|| try_naive_utc(s))
        .ok_or_else(|| {
            Error::InvalidInput(
                "invalid time; use unix seconds or a parseable date/time string".into(),
            )
        })
}

fn try_dateparser(s: &str) -> Option<Timestamp> {
    let dt = dateparser::parse(s).ok()?;
    timestamp_from_utc(dt).ok()
}

fn try_rfc3339(s: &str) -> Option<Timestamp> {
    let dt = DateTime::parse_from_rfc3339(s).ok()?;
    timestamp_from_utc(dt.with_timezone(&Utc)).ok()
}

fn try_naive_utc(s: &str) -> Option<Timestamp> {
    const FMT_SPACE: &str = "%Y-%m-%d %H:%M:%S";
    const FMT_T: &str = "%Y-%m-%dT%H:%M:%S";

    let naive = NaiveDateTime::parse_from_str(s, FMT_SPACE)
        .or_else(|_| NaiveDateTime::parse_from_str(s, FMT_T))
        .ok()?;

    timestamp_from_utc(Utc.from_utc_datetime(&naive)).ok()
}

fn timestamp_from_utc(dt: DateTime<Utc>) -> Result<Timestamp> {
    Timestamp::try_from(dt)
        .map_err(|_| Error::InvalidInput("time must be >= 1970-01-01T00:00:00Z".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_seconds() {
        let t = parse_user_timestamp("1712345678").unwrap();
        assert_eq!(t.as_secs(), 1_712_345_678);
    }

    #[test]
    fn rfc3339_with_offset() {
        let t = parse_user_timestamp("2026-03-14T18:35:59+01:00").unwrap();
        assert!(t.as_secs() > 1_700_000_000);
    }

    #[test]
    fn naive_space_utc() {
        let t = parse_user_timestamp("2026-03-14 18:35:58").unwrap();
        assert_eq!(
            t.as_secs(),
            1_773_513_358,
            "naive datetime interpreted as UTC"
        );
    }

    #[test]
    fn rejects_pre_epoch_rfc3339() {
        assert!(parse_user_timestamp("1969-12-31T23:59:59Z").is_err());
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_user_timestamp("garbage").is_err());
    }
}
