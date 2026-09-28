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

//! Bounded HTTP/1 request parser with Phase 4 known-length body framing.

pub const MAX_REQUEST_LINE: usize = 8192;
pub const MAX_HEADERS: usize = 100;
pub const MAX_HEADER_BYTES: usize = 32_768;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRequest {
    pub method: String,
    pub path: String,
    pub version: HttpVersion,
    pub headers: Vec<Header>,
    /// Absolute byte offset of end of header block (after `\r\n\r\n`).
    pub header_end: usize,
    pub host: Option<String>,
    pub connection_close: bool,
    pub content_length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpVersion {
    Http10,
    Http11,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Incomplete,
    RequestLineTooLong,
    HeaderTooLarge,
    TooManyHeaders,
    MalformedRequestLine,
    MalformedHeader,
    InvalidMethod,
    MissingHost,
    DuplicateHost,
    BodyNotSupported,
    AmbiguousFraming,
    UnsupportedVersion,
}

/// Try to parse a complete HTTP/1 request header block from `buf`.
///
/// Returns `Incomplete` until `\r\n\r\n` is present.
/// Bytes after `header_end` are either the start of a declared CL body or
/// unsupported pipelining; the caller owns that policy.
pub fn try_parse_request(buf: &[u8]) -> Result<ParsedRequest, ParseError> {
    let header_end = find_header_end(buf).ok_or(ParseError::Incomplete)?;
    if header_end > MAX_HEADER_BYTES + MAX_REQUEST_LINE + 4 {
        return Err(ParseError::HeaderTooLarge);
    }
    let head = &buf[..header_end];
    // Fail closed on any non-CRLF line ending. Bare CR can hide Transfer-Encoding
    // from this parser while a desync-prone peer still splits on CR.
    if headers_have_illegal_line_endings(head) {
        return Err(ParseError::MalformedHeader);
    }

    let text = std::str::from_utf8(head).map_err(|_| ParseError::MalformedHeader)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(ParseError::MalformedRequestLine)?;
    if request_line.len() > MAX_REQUEST_LINE {
        return Err(ParseError::RequestLineTooLong);
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(ParseError::MalformedRequestLine)?;
    let path = parts.next().ok_or(ParseError::MalformedRequestLine)?;
    let version_s = parts.next().ok_or(ParseError::MalformedRequestLine)?;
    if parts.next().is_some() {
        return Err(ParseError::MalformedRequestLine);
    }
    if !is_allowed_method(method) {
        return Err(ParseError::InvalidMethod);
    }
    let version = match version_s {
        "HTTP/1.1" => HttpVersion::Http11,
        "HTTP/1.0" => HttpVersion::Http10,
        _ => return Err(ParseError::UnsupportedVersion),
    };

    let mut headers = Vec::new();
    let mut host: Option<String> = None;
    let mut connection_close = version == HttpVersion::Http10;
    let mut saw_te = false;
    let mut content_length: Option<u64> = None;

    for line in lines {
        if line.is_empty() {
            break;
        }
        if headers.len() >= MAX_HEADERS {
            return Err(ParseError::TooManyHeaders);
        }
        let (name, value) = split_header(line)?;
        if name.eq_ignore_ascii_case("host") {
            if host.is_some() {
                return Err(ParseError::DuplicateHost);
            }
            let h = strip_host_port(value.trim());
            if h.is_empty() {
                return Err(ParseError::MissingHost);
            }
            host = Some(h);
        } else if name.eq_ignore_ascii_case("content-length") {
            let n: u64 = value
                .trim()
                .parse()
                .map_err(|_| ParseError::AmbiguousFraming)?;
            if content_length.is_some() {
                return Err(ParseError::AmbiguousFraming);
            }
            content_length = Some(n);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            saw_te = true;
        } else if name.eq_ignore_ascii_case("connection") {
            for tok in value.split(',') {
                let t = tok.trim();
                if t.eq_ignore_ascii_case("close") {
                    connection_close = true;
                } else if t.eq_ignore_ascii_case("keep-alive") {
                    connection_close = false;
                }
            }
        }
        headers.push(Header {
            name: name.to_string(),
            value: value.to_string(),
        });
    }

    if version == HttpVersion::Http11 && host.is_none() {
        return Err(ParseError::MissingHost);
    }
    if saw_te && content_length.is_some() {
        return Err(ParseError::AmbiguousFraming);
    }
    if saw_te {
        return Err(ParseError::BodyNotSupported);
    }

    Ok(ParsedRequest {
        method: method.to_string(),
        path: path.to_string(),
        version,
        headers,
        header_end,
        host,
        connection_close,
        content_length: content_length.unwrap_or(0),
    })
}

fn is_allowed_method(method: &str) -> bool {
    matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD")
}

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

/// True when the header block contains LF without preceding CR, or CR not
/// followed by LF (classic request-smuggling obfuscation vector).
pub(crate) fn headers_have_illegal_line_endings(head: &[u8]) -> bool {
    let mut i = 0;
    while i < head.len() {
        match head[i] {
            b'\r' => {
                if i + 1 >= head.len() || head[i + 1] != b'\n' {
                    return true;
                }
                i += 2;
            }
            b'\n' => return true, // lone LF (no preceding CR; CR LF consumed above)
            _ => i += 1,
        }
    }
    false
}

fn split_header(line: &str) -> Result<(&str, &str), ParseError> {
    let Some(colon) = line.find(':') else {
        return Err(ParseError::MalformedHeader);
    };
    let name = &line[..colon];
    let value = line[colon + 1..].trim_start();
    if name.is_empty() || name.as_bytes().iter().any(|b| *b == b' ' || *b == b'\t') {
        return Err(ParseError::MalformedHeader);
    }
    for b in name.bytes() {
        if !b.is_ascii_alphanumeric() && b != b'-' && b != b'_' {
            return Err(ParseError::MalformedHeader);
        }
    }
    Ok((name, value))
}

/// Strip `:port` from Host (IPv6-aware).
pub fn strip_host_port(host: &str) -> String {
    let host = host.trim();
    if host.starts_with('[') {
        if let Some(end) = host.find(']') {
            return host[..=end].to_string();
        }
        return host.to_string();
    }
    if let Some((h, port)) = host.rsplit_once(':') {
        if port.chars().all(|c| c.is_ascii_digit()) {
            return h.to_string();
        }
    }
    host.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_get() {
        let raw = b"GET /a HTTP/1.1\r\nHost: h.example\r\nConnection: keep-alive\r\n\r\n";
        let p = try_parse_request(raw).unwrap();
        assert_eq!(p.path, "/a");
        assert_eq!(p.host.as_deref(), Some("h.example"));
        assert!(!p.connection_close);
    }

    #[test]
    fn parses_post_without_body() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 0\r\n\r\n";
        let p = try_parse_request(raw).unwrap();
        assert_eq!(p.method, "POST");
        assert_eq!(p.content_length, 0);
    }

    #[test]
    fn parses_known_length_body_framing() {
        let raw = b"PATCH /a HTTP/1.1\r\nHost: h\r\nContent-Length: 5\r\n\r\n";
        let p = try_parse_request(raw).unwrap();
        assert_eq!(p.method, "PATCH");
        assert_eq!(p.content_length, 5);
    }

    #[test]
    fn rejects_transfer_encoding() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::BodyNotSupported));
    }

    #[test]
    fn rejects_cl_te() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn rejects_duplicate_cl() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn rejects_duplicate_host() {
        let raw = b"GET /a HTTP/1.1\r\nHost: a\r\nHost: b\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::DuplicateHost));
    }

    // --- Independent adversarial framing matrix (not copied from phase4_bodies) ---

    #[test]
    fn adversarial_cl_te_order() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn adversarial_te_cl_order() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\nContent-Length: 4\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn adversarial_duplicate_conflicting_cl() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\nContent-Length: 8\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn adversarial_obfuscated_te_space_before_colon() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding : chunked\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::MalformedHeader));
    }

    #[test]
    fn adversarial_obfuscated_te_bare_cr_hides_te() {
        // Without bare-CR rejection this would parse as CL-only (smuggling adjacent).
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 4\r\nX: y\rTransfer-Encoding: chunked\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::MalformedHeader));
    }

    #[test]
    fn adversarial_malformed_te_empty_value() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding:\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::BodyNotSupported));
    }

    #[test]
    fn adversarial_malformed_te_identity_still_rejected() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: identity\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::BodyNotSupported));
    }

    #[test]
    fn adversarial_chunked_alone_fail_closed() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::BodyNotSupported));
    }

    #[test]
    fn adversarial_cl_list_form_rejected() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\r\nContent-Length: 1, 1\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::AmbiguousFraming));
    }

    #[test]
    fn adversarial_lone_lf_rejected() {
        let raw = b"POST /a HTTP/1.1\r\nHost: h\nContent-Length: 1\r\n\r\n";
        assert_eq!(try_parse_request(raw), Err(ParseError::MalformedHeader));
    }

    #[test]
    fn adversarial_known_length_header_end_split() {
        let raw = b"PUT /a HTTP/1.1\r\nHost: h\r\nContent-Length: 3\r\n\r\nabcEXTRA";
        let p = try_parse_request(raw).unwrap();
        assert_eq!(p.content_length, 3);
        assert_eq!(p.header_end, raw.len() - "abcEXTRA".len());
        assert_eq!(&raw[p.header_end..], b"abcEXTRA");
    }
}
