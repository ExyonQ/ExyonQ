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
//! CGI/1.1 stdout → HTTP status/headers/body (sync, no Hyper).

use crate::caps::{MAX_CGI_HEADER_BYTES, MAX_CGI_HEADER_COUNT, MAX_FCGI_RESPONSE_BYTES};

/// CGI stdout parse failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CgiParseError {
    Invalid,
}

/// Parsed CGI success response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CgiResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Parse CGI/1.1 stdout into status + headers + body.
///
/// # Errors
///
/// Returns [`CgiParseError::Invalid`] when framing or header limits fail.
pub fn parse_cgi_stdout(stdout: &[u8]) -> Result<CgiResponse, CgiParseError> {
    if stdout.len() > MAX_FCGI_RESPONSE_BYTES {
        return Err(CgiParseError::Invalid);
    }
    let header_end = stdout
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|pos| pos + 4)
        .or_else(|| {
            stdout
                .windows(2)
                .position(|w| w == b"\n\n")
                .map(|pos| pos + 2)
        })
        .ok_or(CgiParseError::Invalid)?;
    if header_end > MAX_CGI_HEADER_BYTES {
        return Err(CgiParseError::Invalid);
    }
    let header_block = trim_trailing_crlf(&stdout[..header_end]);
    let body = stdout[header_end..].to_vec();
    let header_text = std::str::from_utf8(header_block).map_err(|_| CgiParseError::Invalid)?;
    let nominated = connection_nominated_from_cgi_headers(header_text);
    let mut status = 200u16;
    let mut headers = Vec::new();
    let mut saw_content_length = false;
    for line in header_text.lines() {
        if line.is_empty() {
            continue;
        }
        if headers.len() >= MAX_CGI_HEADER_COUNT {
            return Err(CgiParseError::Invalid);
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(CgiParseError::Invalid);
        };
        let name = name.trim();
        let value = value.trim();
        if name.is_empty() || value.contains('\r') || value.contains('\n') {
            return Err(CgiParseError::Invalid);
        }
        if name.eq_ignore_ascii_case("status") {
            status = parse_status_value(value).ok_or(CgiParseError::Invalid)?;
            continue;
        }
        if is_fixed_hop(name) {
            continue;
        }
        if nominated.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if lower == "content-length" {
            if saw_content_length {
                return Err(CgiParseError::Invalid);
            }
            saw_content_length = true;
        }
        headers.push((lower, value.to_string()));
    }
    Ok(CgiResponse {
        status,
        headers,
        body,
    })
}

fn is_fixed_hop(name: &str) -> bool {
    const FIXED: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "proxy-connection",
        "te",
        "trailer",
        "trailers",
        "transfer-encoding",
        "upgrade",
    ];
    FIXED.iter().any(|h| name.eq_ignore_ascii_case(h))
}

fn connection_nominated_from_cgi_headers(header_text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in header_text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("connection") {
            continue;
        }
        for token in value.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            if token.eq_ignore_ascii_case("close") || token.eq_ignore_ascii_case("keep-alive") {
                continue;
            }
            out.push(token.to_string());
        }
    }
    out
}

fn trim_trailing_crlf(mut bytes: &[u8]) -> &[u8] {
    while bytes.ends_with(b"\r") || bytes.ends_with(b"\n") {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

fn parse_status_value(value: &str) -> Option<u16> {
    let code = value.split_whitespace().next()?.parse().ok()?;
    (100..=599).contains(&code).then_some(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_status_and_body() {
        let r =
            parse_cgi_stdout(b"Status: 201 Created\r\nContent-Type: text/plain\r\n\r\nhi").unwrap();
        assert_eq!(r.status, 201);
        assert_eq!(r.body, b"hi");
    }
}
