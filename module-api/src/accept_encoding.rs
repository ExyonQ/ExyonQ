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
//! Shared Accept-Encoding negotiation (Cap022 / Cap023 / Cap071).
//!
//! Server preference on equal q: zstd > br > gzip > deflate.
//! `q=0` never selected. Wildcard `*` covers unlisted supported codings.
//!
//! Also owns Cap067 wire-cheap **early Hyper gate** for compression: decide from
//! first-request headers only (no mid-connection Cap067→Hyper handoff).

/// Content coding ExyonQ can emit under Cap071.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentCoding {
    Zstd,
    Brotli,
    Gzip,
    Deflate,
}

impl ContentCoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Zstd => "zstd",
            Self::Brotli => "br",
            Self::Gzip => "gzip",
            Self::Deflate => "deflate",
        }
    }

    /// Deterministic server preference (higher = preferred on q tie).
    /// Cap071 owner policy: zstd > br > gzip > deflate.
    fn server_rank(self) -> u8 {
        match self {
            Self::Zstd => 4,
            Self::Brotli => 3,
            Self::Gzip => 2,
            Self::Deflate => 1,
        }
    }
}

/// Full negotiation result — coding preference + whether identity is allowed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Negotiation {
    /// Best supported content coding with q > 0, if any.
    pub coding: Option<ContentCoding>,
    /// q-value of [`Self::coding`] (0.0 when coding is None).
    pub coding_q: f32,
    /// Whether an unencoded (identity) representation is acceptable.
    pub identity_ok: bool,
    /// Effective q for identity (default 1.0 when not listed and `*` absent).
    pub identity_q: f32,
    /// True only when the client listed `identity` explicitly (not the implicit default).
    /// Cap043 ranking vs content-codings applies only when this is true — implicit
    /// identity_q=1.0 must not suppress `gzip;q=0.9` (LA-CAP043-001).
    pub identity_explicit: bool,
}

impl Negotiation {
    /// Resolve what to do when the response may or may not be encodable.
    ///
    /// Cap043: when the client **explicitly** lists identity with q strictly greater
    /// than the best content-coding q, prefer identity (obey client q-values).
    pub fn resolve(self, can_apply_coding: bool) -> NegotiateOutcome {
        if let Some(coding) = self.coding {
            if can_apply_coding {
                if self.identity_explicit
                    && self.identity_ok
                    && self.identity_q > self.coding_q + f32::EPSILON
                {
                    return NegotiateOutcome::Identity;
                }
                return NegotiateOutcome::Encode(coding);
            }
        }
        if self.identity_ok {
            NegotiateOutcome::Identity
        } else {
            NegotiateOutcome::NotAcceptable
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiateOutcome {
    /// Apply this content coding.
    Encode(ContentCoding),
    /// Serve unencoded representation.
    Identity,
    /// Client forbids identity and every supported coding (RFC 9110).
    NotAcceptable,
}

#[derive(Debug, Clone, Copy)]
struct CodingOffer {
    coding: CodingToken,
    q: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodingToken {
    Zstd,
    Brotli,
    Gzip,
    Deflate,
    Identity,
    Star,
    Other,
}

/// Parse Accept-Encoding into coding preference + identity acceptability.
///
/// Missing/empty header → identity OK, no coding preference.
pub fn negotiate(accept_encoding: Option<&str>) -> Negotiation {
    let Some(raw) = accept_encoding else {
        return Negotiation {
            coding: None,
            coding_q: 0.0,
            identity_ok: true,
            identity_q: 1.0,
            identity_explicit: false,
        };
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Negotiation {
            coding: None,
            coding_q: 0.0,
            identity_ok: true,
            identity_q: 1.0,
            identity_explicit: false,
        };
    }

    let offers = parse_offers(trimmed);
    if offers.is_empty() {
        return Negotiation {
            coding: None,
            coding_q: 0.0,
            identity_ok: true,
            identity_q: 1.0,
            identity_explicit: false,
        };
    }

    let q_zstd = effective_q(&offers, CodingToken::Zstd);
    let q_br = effective_q(&offers, CodingToken::Brotli);
    let q_gzip = effective_q(&offers, CodingToken::Gzip);
    let q_deflate = effective_q(&offers, CodingToken::Deflate);
    let q_identity = effective_q(&offers, CodingToken::Identity);

    let mut best: Option<(ContentCoding, f32)> = None;
    for (coding, q) in [
        (ContentCoding::Zstd, q_zstd),
        (ContentCoding::Brotli, q_br),
        (ContentCoding::Gzip, q_gzip),
        (ContentCoding::Deflate, q_deflate),
    ] {
        if q <= 0.0 {
            continue;
        }
        match best {
            None => best = Some((coding, q)),
            Some((prev, prev_q)) => {
                if q > prev_q + f32::EPSILON
                    || ((q - prev_q).abs() <= f32::EPSILON
                        && coding.server_rank() > prev.server_rank())
                {
                    best = Some((coding, q));
                }
            }
        }
    }

    let (coding, coding_q) = match best {
        Some((c, q)) => (Some(c), q),
        None => (None, 0.0),
    };
    let identity_explicit = offers.iter().any(|o| o.coding == CodingToken::Identity);
    Negotiation {
        coding,
        coding_q,
        identity_ok: q_identity > 0.0,
        identity_q: q_identity,
        identity_explicit,
    }
}

fn effective_q(offers: &[CodingOffer], token: CodingToken) -> f32 {
    let mut explicit = None;
    let mut star = None;
    for o in offers {
        match o.coding {
            t if t == token => {
                // Last duplicate wins (HTTP list order).
                explicit = Some(o.q);
            }
            CodingToken::Star => {
                star = Some(o.q);
            }
            _ => {}
        }
    }
    if let Some(q) = explicit {
        return q;
    }
    if token == CodingToken::Identity {
        return star.unwrap_or(1.0);
    }
    star.unwrap_or(0.0)
}

fn parse_offers(raw: &str) -> Vec<CodingOffer> {
    let mut out = Vec::new();
    for part in raw.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let mut name = part;
        let mut q = 1.0_f32;
        if let Some((n, params)) = part.split_once(';') {
            name = n.trim();
            for p in params.split(';') {
                let p = p.trim();
                let Some((k, v)) = p.split_once('=') else {
                    continue;
                };
                if !k.trim().eq_ignore_ascii_case("q") {
                    continue;
                }
                let v = v.trim();
                match parse_q(v) {
                    Some(parsed) => q = parsed,
                    None => q = 0.0,
                }
            }
        }
        let coding = match name.to_ascii_lowercase().as_str() {
            "zstd" => CodingToken::Zstd,
            "br" => CodingToken::Brotli,
            "gzip" | "x-gzip" => CodingToken::Gzip,
            "deflate" => CodingToken::Deflate,
            "identity" => CodingToken::Identity,
            "*" => CodingToken::Star,
            _ => CodingToken::Other,
        };
        out.push(CodingOffer { coding, q });
    }
    out
}

fn parse_q(v: &str) -> Option<f32> {
    let f: f32 = v.parse().ok()?;
    if !(0.0..=1.0).contains(&f) {
        return None;
    }
    Some(f)
}

/// Cap067 early Hyper gate: does Accept-Encoding prefer a supported content coding?
///
/// Missing / empty AE → false (identity). `identity` only → false.
/// Any supported coding with q>0 → true. Fail-closed: header present with tokens
/// that look like coding names but parse to NotAcceptable still returns true when
/// a coding was listed with q>0 via [`negotiate`].
#[inline]
pub fn accept_encoding_prefers_content_coding(accept_encoding: Option<&str>) -> bool {
    negotiate(accept_encoding).coding.is_some()
}

/// Extract raw Accept-Encoding field value from an HTTP/1 request-head byte slice.
pub fn accept_encoding_from_request_head(head: &[u8]) -> Option<&str> {
    for line in head.split(|&b| b == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let Ok(s) = std::str::from_utf8(line) else {
            continue;
        };
        let lower = s.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("accept-encoding:") else {
            continue;
        };
        // Value is ASCII; trim using the same byte length as the lowercased prefix.
        let prefix_len = s.len() - rest.len();
        let val = s.get(prefix_len..)?.trim();
        if !val.is_empty() {
            return Some(val);
        }
    }
    None
}

/// Compression configured → Hyper.
///
/// The static wire does not run the compression module. Leaving it on that
/// wire drops `Vary` and turns `identity;q=0` / `*;q=0` into a raw 200.
#[inline]
pub fn compression_requires_hyper_for_request_head(
    compression_configured: bool,
    _head: &[u8],
) -> bool {
    compression_configured
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(ae: Option<&str>, can_encode: bool) -> NegotiateOutcome {
        negotiate(ae).resolve(can_encode)
    }

    #[test]
    fn missing_and_empty_are_identity() {
        assert_eq!(outcome(None, true), NegotiateOutcome::Identity);
        assert_eq!(outcome(Some(""), true), NegotiateOutcome::Identity);
    }

    #[test]
    fn zstd_only() {
        assert_eq!(
            outcome(Some("zstd"), true),
            NegotiateOutcome::Encode(ContentCoding::Zstd)
        );
    }

    #[test]
    fn br_only() {
        assert_eq!(
            outcome(Some("br"), true),
            NegotiateOutcome::Encode(ContentCoding::Brotli)
        );
    }

    #[test]
    fn deflate_only() {
        assert_eq!(
            outcome(Some("deflate"), true),
            NegotiateOutcome::Encode(ContentCoding::Deflate)
        );
    }

    #[test]
    fn gzip_only() {
        assert_eq!(
            outcome(Some("gzip"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    #[test]
    fn q_values_prefer_higher() {
        assert_eq!(
            outcome(Some("gzip;q=1, br;q=0.5"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
        assert_eq!(
            outcome(Some("br;q=1, zstd;q=0.9"), true),
            NegotiateOutcome::Encode(ContentCoding::Brotli)
        );
        assert_eq!(
            outcome(Some("gzip;q=1, zstd;q=0.8"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
        assert_eq!(
            outcome(Some("deflate;q=1, zstd;q=0"), true),
            NegotiateOutcome::Encode(ContentCoding::Deflate)
        );
    }

    #[test]
    fn q_zero_never_selected() {
        assert_eq!(outcome(Some("zstd;q=0"), true), NegotiateOutcome::Identity);
        assert_eq!(
            outcome(Some("zstd;q=0, gzip;q=1"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
        assert_eq!(
            outcome(Some("gzip;q=1, deflate;q=1, br;q=0, zstd;q=0"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    #[test]
    fn tie_prefers_zstd_then_br_then_gzip() {
        assert_eq!(
            outcome(Some("zstd;q=1, br;q=1, gzip;q=1, deflate;q=1"), true),
            NegotiateOutcome::Encode(ContentCoding::Zstd)
        );
        assert_eq!(
            outcome(Some("gzip, deflate, br"), true),
            NegotiateOutcome::Encode(ContentCoding::Brotli)
        );
        assert_eq!(
            outcome(Some("deflate, gzip"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    #[test]
    fn star_and_identity_rules() {
        assert_eq!(
            outcome(Some("*"), true),
            NegotiateOutcome::Encode(ContentCoding::Zstd)
        );
        assert_eq!(
            outcome(Some("*;q=0"), true),
            NegotiateOutcome::NotAcceptable
        );
        assert_eq!(outcome(Some("identity"), true), NegotiateOutcome::Identity);
        assert_eq!(
            outcome(Some("identity;q=0"), true),
            NegotiateOutcome::NotAcceptable
        );
        assert_eq!(
            outcome(
                Some("zstd;q=0, br;q=0, gzip;q=0, deflate;q=0, identity;q=0"),
                true
            ),
            NegotiateOutcome::NotAcceptable
        );
    }

    #[test]
    fn cannot_encode_with_identity_forbidden_is_406() {
        assert_eq!(
            outcome(Some("zstd, identity;q=0"), false),
            NegotiateOutcome::NotAcceptable
        );
        assert_eq!(
            outcome(Some("zstd, identity;q=0"), true),
            NegotiateOutcome::Encode(ContentCoding::Zstd)
        );
    }

    #[test]
    fn whitespace_and_case() {
        assert_eq!(
            outcome(Some("  Zstd ; q = 1.0  "), true),
            NegotiateOutcome::Encode(ContentCoding::Zstd)
        );
    }

    #[test]
    fn malformed_q_is_zero() {
        assert_eq!(outcome(Some("zstd;q=2"), true), NegotiateOutcome::Identity);
        assert_eq!(
            outcome(Some("zstd;q=abc, gzip"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    #[test]
    fn duplicate_zstd_last_q_wins() {
        assert_eq!(
            outcome(Some("zstd;q=1, zstd;q=0, gzip"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    /// Cap043: client q prefers identity over a lower-q content coding.
    #[test]
    fn identity_q_preferred_over_lower_coding_q() {
        assert_eq!(
            outcome(Some("identity;q=1, gzip;q=0.5"), true),
            NegotiateOutcome::Identity
        );
        assert_eq!(
            outcome(Some("gzip;q=0.1, identity;q=0.9"), true),
            NegotiateOutcome::Identity
        );
        // Equal q: coding still allowed (server may compress).
        assert_eq!(
            outcome(Some("identity;q=1, gzip;q=1"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
    }

    /// LA-CAP043-001: implicit identity must not suppress coding q < 1.
    #[test]
    fn unlisted_identity_does_not_outrank_coding_q() {
        assert_eq!(
            outcome(Some("gzip;q=0.9"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
        assert_eq!(
            outcome(Some("gzip;q=0.5"), true),
            NegotiateOutcome::Encode(ContentCoding::Gzip)
        );
        assert_eq!(
            outcome(Some("br;q=0.2"), true),
            NegotiateOutcome::Encode(ContentCoding::Brotli)
        );
    }

    #[test]
    fn early_gate_ae_idle_false_gzip_true() {
        assert!(!accept_encoding_prefers_content_coding(None));
        assert!(!accept_encoding_prefers_content_coding(Some("identity")));
        assert!(accept_encoding_prefers_content_coding(Some("gzip")));
        assert!(accept_encoding_prefers_content_coding(Some(
            "br, gzip;q=0.8"
        )));
        let head = b"GET /api/ HTTP/1.1\r\nHost: x\r\nAccept-Encoding: gzip\r\n\r\n";
        assert!(compression_requires_hyper_for_request_head(true, head));
        assert!(!compression_requires_hyper_for_request_head(false, head));
        let idle = b"GET /api/ HTTP/1.1\r\nHost: x\r\n\r\n";
        assert!(compression_requires_hyper_for_request_head(true, idle));
        assert!(!compression_requires_hyper_for_request_head(false, idle));
    }
}
