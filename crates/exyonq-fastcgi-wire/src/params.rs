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
//! FastCGI PARAMS builder — SCRIPT_FILENAME authority is caller-supplied.
//!
//! No `exyonq-module-api` dependency: hop-by-hop skip list is inlined (RFC 7230 fixed set).

/// Default `REMOTE_ADDR` when peer address is unavailable.
pub const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1";

/// Fixed hop-by-hop field names (lowercase) — mirrored from module-api SSOT for wire isolation.
const FIXED_HOP_BY_HOP: &[&str] = &[
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

/// PARAMS builder validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamsError {
    EmptyRequestMethod,
    EmptyRequestUri,
    EmptyScriptName,
    EmptyScriptFilename,
    EmptyServerProtocol,
    EmptyDocumentRoot,
    InvalidParamName,
    InvalidParamValue,
    /// Declared Content-Length present but does not equal stdin length (SG-FCGI-04).
    ContentLengthMismatch {
        expected: usize,
        got: usize,
    },
    /// Content-Length declared without matching body length semantics.
    ContentLengthWithoutBody,
    BodyWithoutContentType,
}

/// Forward request fields for PARAMS + STDIN encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinForwardRequest {
    pub request_method: String,
    pub request_uri: String,
    pub script_name: String,
    /// Trusted SCRIPT_FILENAME — never taken from client headers.
    pub script_filename: String,
    pub document_root: String,
    pub server_protocol: String,
    pub server_name: String,
    pub server_port: u16,
    pub query_string: Option<String>,
    pub content_type: Option<String>,
    pub path_info: Option<String>,
    pub remote_addr: String,
    pub http_headers: Vec<(String, String)>,
    pub stdin: Vec<u8>,
    /// When `Some`, must equal `stdin.len()` (SG-FCGI-04).
    pub declared_content_length: Option<usize>,
}

impl MinForwardRequest {
    /// GET with required fields populated.
    pub fn get(request_uri: &str, script_name: &str, script_filename: &str) -> Self {
        Self {
            request_method: "GET".to_string(),
            request_uri: request_uri.to_string(),
            script_name: script_name.to_string(),
            script_filename: script_filename.to_string(),
            document_root: "/var/www".to_string(),
            server_protocol: "HTTP/1.1".to_string(),
            server_name: "localhost".to_string(),
            server_port: 80,
            query_string: None,
            content_type: None,
            path_info: None,
            remote_addr: DEFAULT_REMOTE_ADDR.to_string(),
            http_headers: Vec::new(),
            stdin: Vec::new(),
            declared_content_length: None,
        }
    }

    /// Validate required fields and emit PARAMS pairs.
    ///
    /// # Errors
    ///
    /// Returns [`ParamsError`] on empty required fields, NUL in values, or CL mismatch.
    pub fn to_fcgi_params(&self) -> Result<Vec<(String, String)>, ParamsError> {
        if self.request_method.is_empty() {
            return Err(ParamsError::EmptyRequestMethod);
        }
        if self.request_uri.is_empty() {
            return Err(ParamsError::EmptyRequestUri);
        }
        if self.script_name.is_empty() {
            return Err(ParamsError::EmptyScriptName);
        }
        if self.script_filename.is_empty() {
            return Err(ParamsError::EmptyScriptFilename);
        }
        if self.server_protocol.is_empty() {
            return Err(ParamsError::EmptyServerProtocol);
        }
        if self.document_root.is_empty() {
            return Err(ParamsError::EmptyDocumentRoot);
        }

        // Reject NUL in authority fields before building pairs (caller-supplied SCRIPT_FILENAME).
        validate_param_pair("SCRIPT_FILENAME", &self.script_filename)?;
        validate_param_pair("DOCUMENT_ROOT", &self.document_root)?;
        validate_param_pair("REQUEST_URI", &self.request_uri)?;
        validate_param_pair("SCRIPT_NAME", &self.script_name)?;
        validate_param_pair("REQUEST_METHOD", &self.request_method)?;
        validate_param_pair("SERVER_PROTOCOL", &self.server_protocol)?;
        validate_param_pair("SERVER_NAME", &self.server_name)?;
        validate_param_pair("REMOTE_ADDR", &self.remote_addr)?;

        let stdin_len = self.stdin.len();
        if let Some(declared) = self.declared_content_length {
            if declared != stdin_len {
                return Err(ParamsError::ContentLengthMismatch {
                    expected: declared,
                    got: stdin_len,
                });
            }
        } else if stdin_len > 0 {
            // Body present without declared CL is allowed for encoding, but CONTENT_LENGTH
            // is set from stdin_len below. Content-Type required when body non-empty.
            if self.content_type.as_deref().unwrap_or("").is_empty() {
                return Err(ParamsError::BodyWithoutContentType);
            }
        }

        let mut params = vec![
            ("GATEWAY_INTERFACE".to_string(), "CGI/1.1".to_string()),
            ("REQUEST_METHOD".to_string(), self.request_method.clone()),
            ("REQUEST_URI".to_string(), self.request_uri.clone()),
            ("SCRIPT_NAME".to_string(), self.script_name.clone()),
            ("SCRIPT_FILENAME".to_string(), self.script_filename.clone()),
            ("SERVER_PROTOCOL".to_string(), self.server_protocol.clone()),
            ("DOCUMENT_ROOT".to_string(), self.document_root.clone()),
            ("SERVER_NAME".to_string(), self.server_name.clone()),
            ("SERVER_PORT".to_string(), self.server_port.to_string()),
            ("REMOTE_ADDR".to_string(), self.remote_addr.clone()),
        ];

        if let Some(q) = &self.query_string {
            if !q.is_empty() {
                push_param(&mut params, "QUERY_STRING", q)?;
            }
        }
        if let Some(p) = &self.path_info {
            if !p.is_empty() {
                push_param(&mut params, "PATH_INFO", p)?;
            }
        }
        if stdin_len > 0 || self.declared_content_length == Some(0) {
            push_param(&mut params, "CONTENT_LENGTH", &stdin_len.to_string())?;
            if let Some(ct) = &self.content_type {
                if !ct.is_empty() {
                    push_param(&mut params, "CONTENT_TYPE", ct)?;
                }
            }
        }

        let mut seen = std::collections::HashSet::new();
        for (name, _) in &params {
            seen.insert(name.to_ascii_uppercase());
        }

        let nominated = connection_nominated_names_from_pairs(&self.http_headers);

        for (name, value) in &self.http_headers {
            if nominated
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name.as_str()))
            {
                continue;
            }
            if let Some(cgi_name) = http_header_to_cgi_param(name) {
                if seen.contains(&cgi_name) {
                    continue;
                }
                push_param(&mut params, &cgi_name, value)?;
                seen.insert(cgi_name);
            }
        }

        Ok(params)
    }
}

fn connection_nominated_names_from_pairs(headers: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("connection") {
            continue;
        }
        for token in connection_nominating_tokens(value) {
            out.push(token);
        }
    }
    out
}

fn connection_nominating_tokens(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .filter(|t| !t.eq_ignore_ascii_case("close") && !t.eq_ignore_ascii_case("keep-alive"))
        .map(str::to_string)
        .collect()
}

fn push_param(
    params: &mut Vec<(String, String)>,
    name: &str,
    value: &str,
) -> Result<(), ParamsError> {
    validate_param_pair(name, value)?;
    params.push((name.to_string(), value.to_string()));
    Ok(())
}

fn validate_param_pair(name: &str, value: &str) -> Result<(), ParamsError> {
    if name.is_empty() || name.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n') {
        return Err(ParamsError::InvalidParamName);
    }
    if value.contains('\0') || value.contains('\r') || value.contains('\n') {
        return Err(ParamsError::InvalidParamValue);
    }
    Ok(())
}

fn is_fixed_hop(name: &str) -> bool {
    FIXED_HOP_BY_HOP
        .iter()
        .any(|h| name.eq_ignore_ascii_case(h))
}

/// Map an HTTP header to `HTTP_*` CGI param when safe.
pub fn http_header_to_cgi_param(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if is_fixed_hop(name) {
        return None;
    }
    let upper = name.to_ascii_uppercase().replace('-', "_");
    if matches!(upper.as_str(), "CONTENT_LENGTH" | "CONTENT_TYPE") {
        return None;
    }
    if upper == "HOST" {
        return Some("HTTP_HOST".into());
    }
    if upper.bytes().any(|b| b == 0 || b == b'\r' || b == b'\n') {
        return None;
    }
    Some(format!("HTTP_{upper}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_filename_authority_and_nul_reject() {
        let mut req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
        assert!(req.to_fcgi_params().is_ok());
        req.script_filename = "/var/www/\0evil.php".into();
        assert_eq!(
            req.to_fcgi_params().unwrap_err(),
            ParamsError::InvalidParamValue
        );
    }

    #[test]
    fn content_length_mismatch_sg_fcgi_04() {
        let mut req = MinForwardRequest::get("/i.php", "/i.php", "/var/www/i.php");
        req.stdin = b"abcd".to_vec();
        req.content_type = Some("text/plain".into());
        req.declared_content_length = Some(2);
        assert_eq!(
            req.to_fcgi_params().unwrap_err(),
            ParamsError::ContentLengthMismatch {
                expected: 2,
                got: 4
            }
        );
    }

    #[test]
    fn maps_general_header_to_http_param() {
        assert_eq!(
            http_header_to_cgi_param("X-Custom-Header"),
            Some("HTTP_X_CUSTOM_HEADER".into())
        );
        assert!(http_header_to_cgi_param("Connection").is_none());
    }
}
