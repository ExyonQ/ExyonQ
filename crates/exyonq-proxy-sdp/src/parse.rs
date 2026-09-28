/*
 * Copyright 2026 Antonio Cantallops Alba
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use thiserror::Error;

const MAX_HEADERS: usize = 64;
const MAX_HEADER_BLOCK: usize = 16 * 1024;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseErr {
    #[error("incomplete")]
    Incomplete,
    #[error("reject: {0}")]
    Reject(&'static str),
}

#[derive(Debug, Clone)]
pub struct ClientRequest {
    pub path_query: String,
    pub host: Option<String>,
    pub keepalive: bool,
    pub header_block_len: usize,
}

#[derive(Debug, Clone)]
pub struct UpstreamResponse {
    pub status: u16,
    pub content_length: usize,
    pub keepalive: bool,
    pub header_end: usize,
}

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

pub fn parse_client_request(buf: &[u8]) -> Result<ClientRequest, ParseErr> {
    if buf.len() > MAX_HEADER_BLOCK {
        return Err(ParseErr::Reject("header_too_large"));
    }
    let Some(header_end) = find_header_end(buf) else {
        return Err(ParseErr::Incomplete);
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(&buf[..header_end]) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => return Err(ParseErr::Incomplete),
        Err(_) => return Err(ParseErr::Reject("malformed_request")),
    }
    let method = req.method.ok_or(ParseErr::Reject("no_method"))?;
    if !method.eq_ignore_ascii_case("GET") {
        return Err(ParseErr::Reject("method_not_get"));
    }
    let path = req.path.ok_or(ParseErr::Reject("no_path"))?.to_string();
    let mut host = None;
    let mut cl_count = 0usize;
    let mut has_te = false;
    let mut connection_close = false;
    let mut http11 = true;
    if let Some(v) = req.version {
        http11 = v == 1;
    }
    for h in req.headers.iter() {
        let name = h.name;
        if name.eq_ignore_ascii_case("Host") {
            host = Some(std::str::from_utf8(h.value).unwrap_or("").to_string());
        } else if name.eq_ignore_ascii_case("Content-Length") {
            cl_count += 1;
            let v = std::str::from_utf8(h.value).unwrap_or("");
            if v.trim() != "0" {
                return Err(ParseErr::Reject("get_with_body"));
            }
        } else if name.eq_ignore_ascii_case("Transfer-Encoding") {
            has_te = true;
        } else if name.eq_ignore_ascii_case("Connection") {
            let v = std::str::from_utf8(h.value)
                .unwrap_or("")
                .to_ascii_lowercase();
            if v.split(',').any(|t| t.trim() == "close") {
                connection_close = true;
            }
            if v.split(',').any(|t| t.trim() == "upgrade") {
                return Err(ParseErr::Reject("upgrade"));
            }
        }
    }
    if cl_count > 1 {
        return Err(ParseErr::Reject("duplicate_content_length"));
    }
    if has_te {
        return Err(ParseErr::Reject("transfer_encoding"));
    }
    if has_te && cl_count > 0 {
        return Err(ParseErr::Reject("cl_te"));
    }
    let keepalive = http11 && !connection_close;
    Ok(ClientRequest {
        path_query: path,
        host,
        keepalive,
        header_block_len: header_end,
    })
}

pub fn parse_upstream_response(buf: &[u8]) -> Result<UpstreamResponse, ParseErr> {
    if buf.len() > MAX_HEADER_BLOCK {
        return Err(ParseErr::Reject("upstream_header_too_large"));
    }
    let Some(header_end) = find_header_end(buf) else {
        return Err(ParseErr::Incomplete);
    };
    let mut headers = [httparse::EMPTY_HEADER; MAX_HEADERS];
    let mut res = httparse::Response::new(&mut headers);
    match res.parse(&buf[..header_end]) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => return Err(ParseErr::Incomplete),
        Err(_) => return Err(ParseErr::Reject("malformed_upstream")),
    }
    let status = res.code.ok_or(ParseErr::Reject("no_status"))?;
    let mut cl: Option<usize> = None;
    let mut cl_count = 0usize;
    let mut has_te = false;
    let mut connection_close = false;
    let mut http11 = true;
    if let Some(v) = res.version {
        http11 = v == 1;
    }
    for h in res.headers.iter() {
        if h.name.eq_ignore_ascii_case("Content-Length") {
            cl_count += 1;
            let v = std::str::from_utf8(h.value).unwrap_or("").trim();
            let n: usize = v
                .parse()
                .map_err(|_| ParseErr::Reject("bad_content_length"))?;
            if let Some(prev) = cl {
                if prev != n {
                    return Err(ParseErr::Reject("conflicting_content_length"));
                }
            }
            cl = Some(n);
        } else if h.name.eq_ignore_ascii_case("Transfer-Encoding") {
            has_te = true;
        } else if h.name.eq_ignore_ascii_case("Connection") {
            let v = std::str::from_utf8(h.value)
                .unwrap_or("")
                .to_ascii_lowercase();
            if v.split(',').any(|t| t.trim() == "close") {
                connection_close = true;
            }
        }
    }
    if cl_count > 1 && cl.is_none() {
        return Err(ParseErr::Reject("duplicate_content_length"));
    }
    if has_te {
        // SDP-P1: known-length only — reject chunked (caller closes; no late Hyper).
        return Err(ParseErr::Reject("chunked_unsupported"));
    }
    let content_length = cl.ok_or(ParseErr::Reject("missing_content_length"))?;
    Ok(UpstreamResponse {
        status,
        content_length,
        keepalive: http11 && !connection_close,
        header_end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_te_on_get() {
        let req = b"GET /api/ HTTP/1.1\r\nHost: a\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(matches!(
            parse_client_request(req),
            Err(ParseErr::Reject("transfer_encoding"))
        ));
    }

    #[test]
    fn rejects_duplicate_cl() {
        let req =
            b"GET /api/ HTTP/1.1\r\nHost: a\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n";
        assert!(matches!(
            parse_client_request(req),
            Err(ParseErr::Reject("duplicate_content_length"))
        ));
    }

    #[test]
    fn parses_simple_get() {
        let req = b"GET /api/ HTTP/1.1\r\nHost: example\r\n\r\n";
        let p = parse_client_request(req).expect("ok");
        assert_eq!(p.path_query, "/api/");
        assert!(p.keepalive);
    }
}
