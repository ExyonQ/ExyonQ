/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */
//! Strict RFC 3339 parser for config timestamps (`[waf.ip] until`).
//!
//! No new dependency: config timestamps are one unambiguous instant, so the
//! parser accepts exactly `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` and rejects
//! everything else (no local-time guessing, no lenient separators).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A config timestamp that is not an unambiguous RFC 3339 instant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampError;

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an RFC 3339 UTC timestamp")
    }
}

impl std::error::Error for TimestampError {}

/// Parse an RFC 3339 timestamp into a UTC [`SystemTime`].
///
/// `Err(TimestampError)` means "not a valid RFC 3339 instant"; callers turn
/// that into a config error instead of falling back to `now` or an unbounded
/// entry.
pub fn parse_rfc3339_utc(raw: &str) -> Result<SystemTime, TimestampError> {
    let s = raw.trim();
    let b = s.as_bytes();
    if b.len() < 20 {
        return Err(TimestampError);
    }
    if b[4] != b'-'
        || b[7] != b'-'
        || !matches!(b[10], b'T' | b't')
        || b[13] != b':'
        || b[16] != b':'
    {
        return Err(TimestampError);
    }

    fn num(b: &[u8], range: std::ops::Range<usize>) -> Result<i64, TimestampError> {
        let mut value = 0i64;
        for i in range {
            let c = *b.get(i).ok_or(TimestampError)?;
            if !c.is_ascii_digit() {
                return Err(TimestampError);
            }
            value = value * 10 + i64::from(c - b'0');
        }
        Ok(value)
    }

    let year = num(b, 0..4)?;
    let month = num(b, 5..7)?;
    let day = num(b, 8..10)?;
    let hour = num(b, 11..13)?;
    let minute = num(b, 14..16)?;
    let second = num(b, 17..19)?;
    if year < 1970
        || !(1..=12).contains(&month)
        || day < 1
        || day > i64::from(days_in_month(year as u32, month as u32))
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(TimestampError);
    }

    let mut i = 19;
    let mut nanos = 0u32;
    if b.get(i) == Some(&b'.') {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return Err(TimestampError);
        }
        let mut digits: String = s[start..i].chars().take(9).collect();
        while digits.len() < 9 {
            digits.push('0');
        }
        nanos = digits.parse().map_err(|_| TimestampError)?;
    }

    let rest = s.get(i..).ok_or(TimestampError)?;
    let offset_secs = match rest {
        "Z" | "z" => 0,
        rest if rest.len() == 6
            && (rest.starts_with('+') || rest.starts_with('-'))
            && rest.as_bytes()[3] == b':' =>
        {
            let hh: i64 = rest[1..3].parse().map_err(|_| TimestampError)?;
            let mm: i64 = rest[4..6].parse().map_err(|_| TimestampError)?;
            if hh > 23 || mm > 59 {
                return Err(TimestampError);
            }
            let secs = hh * 3600 + mm * 60;
            if rest.starts_with('-') {
                -secs
            } else {
                secs
            }
        }
        _ => return Err(TimestampError),
    };

    let days = days_from_civil(year, month, day);
    let secs = days * 86_400 + hour * 3600 + minute * 60 + second - offset_secs;
    if secs < 0 {
        return Err(TimestampError);
    }
    Ok(UNIX_EPOCH + Duration::new(secs as u64, nanos))
}

fn is_leap(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 (Howard Hinnant's civil-from-days inverse).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = (month + 9) % 12;
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_and_known_instants() {
        assert_eq!(
            parse_rfc3339_utc("1970-01-01T00:00:00Z").unwrap(),
            UNIX_EPOCH
        );
        assert_eq!(
            parse_rfc3339_utc("2026-11-14T08:30:00Z").unwrap(),
            UNIX_EPOCH + Duration::from_secs(1_794_645_000)
        );
        assert_eq!(
            parse_rfc3339_utc("2024-02-29T23:59:59.5Z").unwrap(),
            UNIX_EPOCH + Duration::new(1_709_251_199, 500_000_000)
        );
    }

    #[test]
    fn offsets_are_applied() {
        let base = parse_rfc3339_utc("2026-11-14T08:30:00Z").unwrap();
        assert_eq!(
            parse_rfc3339_utc("2026-11-14T09:30:00+01:00").unwrap(),
            base
        );
        assert_eq!(
            parse_rfc3339_utc("2026-11-14T03:30:00-05:00").unwrap(),
            base
        );
    }

    #[test]
    fn rejects_ambiguous_or_malformed() {
        for raw in [
            "",
            "2026-11-14",
            "2026-11-14 08:30:00Z",
            "2026-11-14T08:30:00",
            "2026-13-01T00:00:00Z",
            "2025-02-29T00:00:00Z",
            "2026-11-14T24:00:00Z",
            "2026-11-14T08:30:60Z",
            "2026-11-14T08:30:00.Z",
            "2026-11-14T08:30:00+1:00",
        ] {
            assert!(parse_rfc3339_utc(raw).is_err(), "`{raw}` must be rejected");
        }
    }
}
