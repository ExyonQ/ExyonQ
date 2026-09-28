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
//! Cap020 — weak ETag / Last-Modified + If-None-Match / If-Modified-Since → 304.
//!
//! Representation scope: unencoded static resource (compatible with Cap019 Range).

use crate::identity::StaticResourceIdentity;
use std::time::{SystemTime, UNIX_EPOCH};

/// Validators derived from Cap004-authorized resource identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticValidators {
    /// Wire value including weak prefix, e.g. `W/"exq-…"`.
    pub etag: String,
    /// IMF-fixdate GMT, if mtime available.
    pub last_modified: Option<String>,
    /// Truncated to whole seconds for IMS comparison.
    pub mtime_secs: Option<u64>,
}

impl StaticValidators {
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        Self::from_identity(&StaticResourceIdentity::from_metadata(
            std::path::Path::new(""),
            meta,
        ))
    }

    /// Cap020 validators from a Cap067 `fstatat` identity (encoding-cache hit path).
    pub fn from_validator_identity(id: &ValidatorIdentity) -> Self {
        #[cfg(unix)]
        let (dev, ino) = (id.dev, id.ino);
        #[cfg(not(unix))]
        let (dev, ino) = (0u64, 0u64);
        let etag = format!(
            "W/\"exq-{dev}-{ino}-{}-{}-{}\"",
            id.mtime_secs, id.mtime_nsecs, id.len
        );
        let last_modified = Some(format_http_date_secs(id.mtime_secs));
        Self {
            etag,
            last_modified,
            mtime_secs: Some(id.mtime_secs),
        }
    }

    pub fn from_identity(id: &StaticResourceIdentity) -> Self {
        let (dev, ino) = match id.platform_file_id {
            #[cfg(unix)]
            Some(fid) => (fid.dev, fid.ino),
            #[cfg(not(unix))]
            Some(_) => (0, 0),
            None => (0, 0),
        };
        let (mtime_secs, mtime_nsecs) = match id.modified {
            Some(t) => match t.duration_since(UNIX_EPOCH) {
                Ok(d) => (d.as_secs(), d.subsec_nanos()),
                Err(_) => (0, 0),
            },
            None => (0, 0),
        };
        let etag = format!(
            "W/\"exq-{dev}-{ino}-{mtime_secs}-{mtime_nsecs}-{}\"",
            id.file_len
        );
        let last_modified = id.modified.and_then(format_http_date);
        let mtime_secs_opt = id
            .modified
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs());
        Self {
            etag,
            last_modified,
            mtime_secs: mtime_secs_opt,
        }
    }

    pub fn append_to_headers(&self, headers: &mut Vec<(String, String)>) {
        headers.push(("etag".into(), self.etag.clone()));
        if let Some(lm) = &self.last_modified {
            headers.push(("last-modified".into(), lm.clone()));
        }
    }
}

/// File identity used to decide whether generation-scoped prepared validators/headers
/// may be reused after Cap004 open (must match opened fd metadata).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorIdentity {
    #[cfg(unix)]
    pub dev: u64,
    #[cfg(unix)]
    pub ino: u64,
    pub len: u64,
    pub mtime_secs: u64,
    pub mtime_nsecs: u32,
}

impl ValidatorIdentity {
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let (mtime_secs, mtime_nsecs) = match meta.modified() {
                Ok(t) => match t.duration_since(UNIX_EPOCH) {
                    Ok(d) => (d.as_secs(), d.subsec_nanos()),
                    Err(_) => (0, 0),
                },
                Err(_) => (0, 0),
            };
            Self {
                dev: meta.dev(),
                ino: meta.ino(),
                len: meta.len(),
                mtime_secs,
                mtime_nsecs,
            }
        }
        #[cfg(not(unix))]
        {
            let (mtime_secs, mtime_nsecs) = match meta.modified() {
                Ok(t) => match t.duration_since(UNIX_EPOCH) {
                    Ok(d) => (d.as_secs(), d.subsec_nanos()),
                    Err(_) => (0, 0),
                },
                Err(_) => (0, 0),
            };
            Self {
                len: meta.len(),
                mtime_secs,
                mtime_nsecs,
            }
        }
    }

    #[inline]
    pub fn matches_metadata(&self, meta: &std::fs::Metadata) -> bool {
        *self == Self::from_metadata(meta)
    }
}

/// Generation-scoped Cap020 wire fragments for Cap067 sendfile (immutable until identity changes).
#[derive(Debug, Clone)]
pub struct PreparedStaticWire {
    pub identity: ValidatorIdentity,
    pub validators: std::sync::Arc<StaticValidators>,
    pub header_200: std::sync::Arc<bytes::Bytes>,
    pub header_304: std::sync::Arc<bytes::Bytes>,
}

impl PreparedStaticWire {
    pub fn from_metadata(meta: &std::fs::Metadata, content_type: &'static str) -> Self {
        let identity = ValidatorIdentity::from_metadata(meta);
        let validators = std::sync::Arc::new(StaticValidators::from_metadata(meta));
        let len = usize::try_from(meta.len()).unwrap_or(usize::MAX);
        let header_200 = std::sync::Arc::new(crate::wire::ok_header_with_validators(
            len,
            content_type,
            Some(&validators.etag),
            validators.last_modified.as_deref(),
        ));
        let header_304 = std::sync::Arc::new(crate::wire::not_modified_header(
            &validators.etag,
            validators.last_modified.as_deref(),
        ));
        Self {
            identity,
            validators,
            header_200,
            header_304,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConditionalDecision {
    /// Serve representation (200/206/416 path continues).
    Continue,
    /// Validators match → 304 Not Modified.
    NotModified,
}

/// Cap020 decision. INM present (evaluable) takes precedence over IMS.
pub fn decide_conditional(
    if_none_match: Option<&str>,
    if_modified_since: Option<&str>,
    validators: &StaticValidators,
) -> ConditionalDecision {
    match evaluate_if_none_match(if_none_match, &validators.etag) {
        InmResult::PresentMatch => return ConditionalDecision::NotModified,
        InmResult::PresentNoMatch => return ConditionalDecision::Continue,
        InmResult::AbsentOrIgnored => {}
    }
    if evaluate_if_modified_since(if_modified_since, validators.mtime_secs) {
        return ConditionalDecision::NotModified;
    }
    ConditionalDecision::Continue
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InmResult {
    AbsentOrIgnored,
    PresentMatch,
    PresentNoMatch,
}

fn evaluate_if_none_match(raw: Option<&str>, current_etag: &str) -> InmResult {
    let Some(raw) = raw else {
        return InmResult::AbsentOrIgnored;
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return InmResult::AbsentOrIgnored;
    }
    if trimmed == "*" {
        return InmResult::PresentMatch;
    }
    let Some(tags) = parse_entity_tag_list(trimmed) else {
        return InmResult::AbsentOrIgnored;
    };
    if tags.is_empty() {
        return InmResult::AbsentOrIgnored;
    }
    let current = normalize_etag_for_weak_compare(current_etag);
    for tag in tags {
        if tag == "*" {
            return InmResult::PresentMatch;
        }
        if normalize_etag_for_weak_compare(&tag) == current {
            return InmResult::PresentMatch;
        }
    }
    InmResult::PresentNoMatch
}

/// Weak comparison: strip optional `W/` and compare opaque-tag including quotes.
fn normalize_etag_for_weak_compare(tag: &str) -> String {
    let t = tag.trim();
    if let Some(rest) = t.strip_prefix("W/") {
        rest.trim().to_string()
    } else if let Some(rest) = t.strip_prefix("w/") {
        rest.trim().to_string()
    } else {
        t.to_string()
    }
}

/// Parse a single If-None-Match field value into entity-tags.
/// Returns `None` on malformed grammar (caller ignores INM).
fn parse_entity_tag_list(raw: &str) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut rest = raw.trim();
    if rest.is_empty() {
        return None;
    }
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if rest.starts_with('*') {
            out.push("*".into());
            rest = &rest[1..];
        } else {
            let weak = if let Some(r) = rest.strip_prefix("W/").or_else(|| rest.strip_prefix("w/"))
            {
                rest = r;
                true
            } else {
                false
            };
            rest = rest.trim_start();
            if !rest.starts_with('"') {
                return None;
            }
            rest = &rest[1..];
            let mut opaque = String::new();
            let mut closed = false;
            while let Some(ch) = rest.chars().next() {
                if ch == '"' {
                    rest = &rest[1..];
                    closed = true;
                    break;
                }
                if ch == '\\' {
                    rest = &rest[1..];
                    let mut chars = rest.chars();
                    let esc = chars.next()?;
                    opaque.push(esc);
                    rest = chars.as_str();
                    continue;
                }
                if ch.is_control() {
                    return None;
                }
                opaque.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
            if !closed {
                return None;
            }
            out.push(if weak {
                format!("W/\"{opaque}\"")
            } else {
                format!("\"{opaque}\"")
            });
        }
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        if !rest.starts_with(',') {
            return None;
        }
        rest = rest[1..].trim_start();
        if rest.is_empty() {
            return None;
        }
    }
    Some(out)
}

fn evaluate_if_modified_since(raw: Option<&str>, mtime_secs: Option<u64>) -> bool {
    let (Some(raw), Some(mtime_secs)) = (raw, mtime_secs) else {
        return false;
    };
    let Some(ims) = parse_http_date(raw.trim()) else {
        return false;
    };
    // RFC 9110: if Last-Modified equals or is earlier than IMS → not modified.
    mtime_secs <= ims
}

/// Extract first header value (case-insensitive). Multiple → `None` (ignore).
pub fn single_header_value<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    let mut found: Option<&str> = None;
    for (n, v) in headers {
        if n.eq_ignore_ascii_case(name) {
            if found.is_some() {
                return None;
            }
            found = Some(v.as_str());
        }
    }
    found
}

/// Extract header from raw HTTP/1 request head. Multiple → `None`.
pub fn header_from_raw_head<'a>(head: &'a [u8], name: &[u8]) -> Option<&'a str> {
    let mut found: Option<&str> = None;
    let mut rest = head;
    let nl = rest.iter().position(|&b| b == b'\n')?;
    rest = &rest[nl + 1..];
    while !rest.is_empty() {
        if rest.starts_with(b"\r\n") || rest.starts_with(b"\n") {
            break;
        }
        let line_end = rest.iter().position(|&b| b == b'\n').unwrap_or(rest.len());
        let mut line = &rest[..line_end];
        if let Some(stripped) = line.strip_suffix(b"\r") {
            line = stripped;
        }
        rest = rest.get(line_end + 1..).unwrap_or(&[]);
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let hname = &line[..colon];
        if !hname.eq_ignore_ascii_case(name) {
            continue;
        }
        let mut value = &line[colon + 1..];
        while value.first() == Some(&b' ') || value.first() == Some(&b'\t') {
            value = &value[1..];
        }
        let Ok(s) = std::str::from_utf8(value) else {
            return None;
        };
        if found.is_some() {
            return None;
        }
        found = Some(s);
    }
    found
}

pub fn format_http_date(t: SystemTime) -> Option<String> {
    let d = t.duration_since(UNIX_EPOCH).ok()?;
    Some(format_http_date_secs(d.as_secs()))
}

fn format_http_date_secs(secs: u64) -> String {
    // Algorithm from civil time conversion (Howard Hinnant), UTC.
    let days = (secs / 86_400) as i64;
    let tod = (secs % 86_400) as u32;
    let hour = tod / 3600;
    let min = (tod % 3600) / 60;
    let sec = tod % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    // 1970-01-01 was Thursday → days=0 → "Thu".
    let weekday = days.rem_euclid(7) as usize;
    const WK: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MON: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mon = MON[(m as usize).saturating_sub(1).min(11)];
    format!(
        "{}, {:02} {} {:04} {:02}:{:02}:{:02} GMT",
        WK[weekday], d, mon, y, hour, min, sec
    )
}

pub fn parse_http_date(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(v) = parse_imf_fixdate(s) {
        return Some(v);
    }
    if let Some(v) = parse_rfc850(s) {
        return Some(v);
    }
    parse_asctime(s)
}

fn parse_imf_fixdate(s: &str) -> Option<u64> {
    // Sun, 06 Nov 1994 08:49:37 GMT
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 6 || !parts[5].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let day: u32 = parts[1].parse().ok()?;
    let month = month_num(parts[2])?;
    let year: i32 = parts[3].parse().ok()?;
    let (hh, mm, ss) = parse_hms(parts[4])?;
    ymd_hms_to_unix(year, month, day, hh, mm, ss)
}

fn parse_rfc850(s: &str) -> Option<u64> {
    // Sunday, 06-Nov-94 08:49:37 GMT
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 4 || !parts[3].eq_ignore_ascii_case("GMT") {
        return None;
    }
    let date = parts[1];
    let mut segs = date.split('-');
    let day: u32 = segs.next()?.parse().ok()?;
    let month = month_num(segs.next()?)?;
    let yy: i32 = segs.next()?.parse().ok()?;
    let year = if yy >= 70 { 1900 + yy } else { 2000 + yy };
    let (hh, mm, ss) = parse_hms(parts[2])?;
    ymd_hms_to_unix(year, month, day, hh, mm, ss)
}

fn parse_asctime(s: &str) -> Option<u64> {
    // Sun Nov  6 08:49:37 1994
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let month = month_num(parts[1])?;
    let day: u32 = parts[2].parse().ok()?;
    let (hh, mm, ss) = parse_hms(parts[3])?;
    let year: i32 = parts[4].parse().ok()?;
    ymd_hms_to_unix(year, month, day, hh, mm, ss)
}

fn parse_hms(s: &str) -> Option<(u32, u32, u32)> {
    let mut p = s.split(':');
    let hh: u32 = p.next()?.parse().ok()?;
    let mm: u32 = p.next()?.parse().ok()?;
    let ss: u32 = p.next()?.parse().ok()?;
    if p.next().is_some() || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    Some((hh, mm, ss))
}

fn month_num(m: &str) -> Option<u32> {
    match m {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
            if leap {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn ymd_hms_to_unix(year: i32, month: u32, day: u32, hh: u32, mm: u32, ss: u32) -> Option<u64> {
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    // Days from civil (Hinnant)
    let y = if month <= 2 { year - 1 } else { year };
    let m = month as i64;
    let d = day as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy as u64;
    let days = (era as i64) * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return None;
    }
    let tod = (hh as u64) * 3600 + (mm as u64) * 60 + ss as u64;
    Some(days as u64 * 86_400 + tod)
}

/// 304 outcome headers (no body).
pub fn not_modified_headers(validators: &StaticValidators) -> Vec<(String, String)> {
    let mut h = Vec::with_capacity(2);
    validators.append_to_headers(&mut h);
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::StaticResourceIdentity;
    use std::fs;
    use std::path::Path;

    fn id_for(path: &Path) -> StaticResourceIdentity {
        StaticResourceIdentity::capture(path).expect("capture")
    }

    #[test]
    fn etag_weak_and_stable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").unwrap();
        let v1 = StaticValidators::from_identity(&id_for(&path));
        assert!(v1.etag.starts_with("W/\"exq-"));
        let v2 = StaticValidators::from_identity(&id_for(&path));
        assert_eq!(v1.etag, v2.etag);
    }

    #[test]
    fn inm_match_and_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").unwrap();
        let v = StaticValidators::from_identity(&id_for(&path));
        assert_eq!(
            decide_conditional(Some(&v.etag), None, &v),
            ConditionalDecision::NotModified
        );
        assert_eq!(
            decide_conditional(Some("W/\"other\""), v.last_modified.as_deref(), &v),
            ConditionalDecision::Continue
        );
        assert_eq!(
            decide_conditional(Some("*"), None, &v),
            ConditionalDecision::NotModified
        );
    }

    #[test]
    fn ims_match_without_inm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").unwrap();
        let v = StaticValidators::from_identity(&id_for(&path));
        let lm = v.last_modified.clone().expect("lm");
        assert_eq!(
            decide_conditional(None, Some(&lm), &v),
            ConditionalDecision::NotModified
        );
        assert_eq!(
            decide_conditional(None, Some("Thu, 01 Jan 1970 00:00:00 GMT"), &v),
            ConditionalDecision::Continue
        );
    }

    #[test]
    fn malformed_ignored() {
        let v = StaticValidators {
            etag: "W/\"exq-0-0-0-0-1\"".into(),
            last_modified: Some("Thu, 01 Jan 1970 00:00:01 GMT".into()),
            mtime_secs: Some(1),
        };
        assert_eq!(
            decide_conditional(Some("not-a-tag"), Some("not-a-date"), &v),
            ConditionalDecision::Continue
        );
    }

    #[test]
    fn http_date_roundtrip() {
        let s = format_http_date_secs(784_111_777); // 1994-11-06 08:49:37 UTC
        assert_eq!(s, "Sun, 06 Nov 1994 08:49:37 GMT");
        assert_eq!(parse_http_date(&s), Some(784_111_777));
    }

    #[test]
    fn invalid_calendar_ims_ignored() {
        let v = StaticValidators {
            etag: "W/\"exq-0-0-0-0-1\"".into(),
            last_modified: Some("Thu, 01 Jan 1970 00:00:01 GMT".into()),
            mtime_secs: Some(1),
        };
        assert_eq!(
            decide_conditional(None, Some("Thu, 31 Feb 2020 00:00:00 GMT"), &v),
            ConditionalDecision::Continue
        );
    }
}
