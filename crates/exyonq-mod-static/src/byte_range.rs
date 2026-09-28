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
//! Cap019 — single-byte-range parse + satisfy (RFC 9110 subset).
//!
//! Unsupported / malformed → [`RangeDecision::Ignore`] (caller serves full 200).
//! Valid unsatisfiable → [`RangeDecision::Unsatisfiable`].
//! Valid satisfiable → [`RangeDecision::Satisfied`] with inclusive start/end.

/// Inclusive selected byte interval against representation length `N`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectedRange {
    pub start: u64,
    pub end: u64,
    pub full_length: u64,
}

impl SelectedRange {
    pub fn content_length(self) -> u64 {
        self.end - self.start + 1
    }

    pub fn content_range_value(self) -> String {
        format!("bytes {}-{}/{}", self.start, self.end, self.full_length)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeDecision {
    /// No Range header, or malformed / unsupported → serve full entity.
    Ignore,
    Unsatisfiable {
        full_length: u64,
    },
    Satisfied(SelectedRange),
}

/// Extract `Range` from a raw HTTP/1 request head (request-line + headers).
/// Multiple `Range` headers → `None` (Ignore).
pub fn range_header_from_raw_head(head: &[u8]) -> Option<&str> {
    let mut found: Option<&str> = None;
    let mut rest = head;
    // Skip request line.
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
        let name = &line[..colon];
        if !name.eq_ignore_ascii_case(b"range") {
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

/// Extract the first `Range` header value (case-insensitive name). Multiple Range
/// headers → Ignore (do not merge).
pub fn range_header_value(headers: &[(String, String)]) -> Option<&str> {
    let mut found: Option<&str> = None;
    for (name, value) in headers {
        if name.eq_ignore_ascii_case("range") {
            if found.is_some() {
                return None;
            }
            found = Some(value.as_str());
        }
    }
    found
}

/// Cap019 v1 decision from raw `Range` header value and representation length.
pub fn decide_range(range_value: Option<&str>, full_length: u64) -> RangeDecision {
    let Some(raw) = range_value else {
        return RangeDecision::Ignore;
    };
    let Some(spec) = parse_single_bytes_range(raw) else {
        return RangeDecision::Ignore;
    };
    satisfy(spec, full_length)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spec {
    Inclusive { start: u64, end: u64 },
    FromStart { start: u64 },
    Suffix { length: u64 },
}

fn parse_single_bytes_range(raw: &str) -> Option<Spec> {
    let trimmed = raw.trim();
    let rest = trimmed.strip_prefix("bytes")?;
    // Cap019: optional whitespace around '='.
    let rest = rest.trim_start().strip_prefix('=')?.trim_start();
    if rest.is_empty() {
        return None;
    }
    // Multi-range: any comma → unsupported v1.
    if rest.contains(',') {
        return None;
    }
    if let Some(suffix) = rest.strip_prefix('-') {
        let length = parse_u64_strict(suffix.trim())?;
        if length == 0 {
            return None;
        }
        return Some(Spec::Suffix { length });
    }
    let (start_s, end_s) = rest.split_once('-')?;
    let start = parse_u64_strict(start_s.trim())?;
    let end_s = end_s.trim();
    if end_s.is_empty() {
        return Some(Spec::FromStart { start });
    }
    let end = parse_u64_strict(end_s)?;
    if start > end {
        return None;
    }
    Some(Spec::Inclusive { start, end })
}

fn parse_u64_strict(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Reject leading zeros that are not the single digit 0 (optional strictness).
    // Accept "0" and "01" as decimal — RFC allows; overflow still fails.
    s.parse::<u64>().ok()
}

fn satisfy(spec: Spec, n: u64) -> RangeDecision {
    match spec {
        Spec::Inclusive { start, end } => {
            if start >= n {
                return RangeDecision::Unsatisfiable { full_length: n };
            }
            let end = end.min(n.saturating_sub(1));
            RangeDecision::Satisfied(SelectedRange {
                start,
                end,
                full_length: n,
            })
        }
        Spec::FromStart { start } => {
            if start >= n {
                return RangeDecision::Unsatisfiable { full_length: n };
            }
            RangeDecision::Satisfied(SelectedRange {
                start,
                end: n - 1,
                full_length: n,
            })
        }
        Spec::Suffix { length } => {
            if n == 0 {
                return RangeDecision::Unsatisfiable { full_length: 0 };
            }
            let start = n.saturating_sub(length);
            RangeDecision::Satisfied(SelectedRange {
                start,
                end: n - 1,
                full_length: n,
            })
        }
    }
}

/// Build Cap019 206/416 outcome headers (lowercase names match existing static outcomes).
#[allow(clippy::type_complexity)] // (status, headers, body_len) tuple is the wire outcome contract
pub fn range_response_headers(
    content_type: &str,
    decision: RangeDecision,
    head_only: bool,
) -> Option<(u16, Vec<(String, String)>, u64)> {
    match decision {
        RangeDecision::Ignore => None,
        RangeDecision::Unsatisfiable { full_length } => Some((
            416,
            vec![
                ("accept-ranges".into(), "bytes".into()),
                ("content-range".into(), format!("bytes */{full_length}")),
                ("content-length".into(), "0".into()),
                ("content-type".into(), content_type.to_string()),
            ],
            0,
        )),
        RangeDecision::Satisfied(sel) => {
            let len = sel.content_length();
            let _ = head_only;
            Some((
                206,
                vec![
                    ("accept-ranges".into(), "bytes".into()),
                    ("content-range".into(), sel.content_range_value()),
                    ("content-length".into(), len.to_string()),
                    ("content-type".into(), content_type.to_string()),
                ],
                len,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inclusive_and_clip() {
        assert_eq!(
            decide_range(Some("bytes=0-99"), 1000),
            RangeDecision::Satisfied(SelectedRange {
                start: 0,
                end: 99,
                full_length: 1000
            })
        );
        assert_eq!(
            decide_range(Some("bytes=0-9999"), 1000),
            RangeDecision::Satisfied(SelectedRange {
                start: 0,
                end: 999,
                full_length: 1000
            })
        );
    }

    #[test]
    fn open_end_and_suffix() {
        assert_eq!(
            decide_range(Some("bytes=100-"), 1000),
            RangeDecision::Satisfied(SelectedRange {
                start: 100,
                end: 999,
                full_length: 1000
            })
        );
        assert_eq!(
            decide_range(Some("bytes=-100"), 1000),
            RangeDecision::Satisfied(SelectedRange {
                start: 900,
                end: 999,
                full_length: 1000
            })
        );
        assert_eq!(
            decide_range(Some("bytes=-5000"), 1000),
            RangeDecision::Satisfied(SelectedRange {
                start: 0,
                end: 999,
                full_length: 1000
            })
        );
    }

    #[test]
    fn unsatisfiable() {
        assert_eq!(
            decide_range(Some("bytes=1000-1001"), 1000),
            RangeDecision::Unsatisfiable { full_length: 1000 }
        );
        assert_eq!(
            decide_range(Some("bytes=5000-"), 1000),
            RangeDecision::Unsatisfiable { full_length: 1000 }
        );
        assert_eq!(
            decide_range(Some("bytes=0-0"), 0),
            RangeDecision::Unsatisfiable { full_length: 0 }
        );
        assert_eq!(
            decide_range(Some("bytes=-1"), 0),
            RangeDecision::Unsatisfiable { full_length: 0 }
        );
    }

    #[test]
    fn ignore_malformed_and_multi() {
        assert_eq!(decide_range(Some("items=0-10"), 100), RangeDecision::Ignore);
        assert_eq!(
            decide_range(Some("bytes=abc-def"), 100),
            RangeDecision::Ignore
        );
        assert_eq!(decide_range(Some("bytes="), 100), RangeDecision::Ignore);
        assert_eq!(
            decide_range(Some("bytes=0-1,4-5"), 100),
            RangeDecision::Ignore
        );
        assert_eq!(decide_range(Some("bytes=10-5"), 100), RangeDecision::Ignore);
        assert_eq!(decide_range(Some("bytes=-0"), 100), RangeDecision::Ignore);
        assert_eq!(
            decide_range(Some("bytes=18446744073709551616-"), 100),
            RangeDecision::Ignore
        );
        assert_eq!(
            decide_range(Some("bytes =0-10"), 100),
            RangeDecision::Satisfied(SelectedRange {
                start: 0,
                end: 10,
                full_length: 100
            })
        );
    }

    #[test]
    fn one_byte_file() {
        assert_eq!(
            decide_range(Some("bytes=0-0"), 1),
            RangeDecision::Satisfied(SelectedRange {
                start: 0,
                end: 0,
                full_length: 1
            })
        );
        assert_eq!(
            decide_range(Some("bytes=1-"), 1),
            RangeDecision::Unsatisfiable { full_length: 1 }
        );
    }

    #[test]
    fn multi_range_header_names() {
        assert!(range_header_value(&[
            ("Range".into(), "bytes=0-1".into()),
            ("range".into(), "bytes=2-3".into()),
        ])
        .is_none());
        assert_eq!(
            range_header_value(&[("Range".into(), "bytes=0-1".into())]),
            Some("bytes=0-1")
        );
    }
}
