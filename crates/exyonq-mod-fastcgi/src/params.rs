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
//! FastCGI PARAMS builder — PR5-B1-real production mapping.

/// Default `REMOTE_ADDR` for structural mapping when peer address is unavailable.
pub const DEFAULT_REMOTE_ADDR: &str = "127.0.0.1";

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
    ContentLengthWithoutBody,
    BodyWithoutContentLength,
    ContentLengthMismatch { expected: usize, got: usize },
}

/// Forward request fields for PARAMS + STDIN encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinForwardRequest {
    pub request_method: String,
    pub request_uri: String,
    pub script_name: String,
    pub script_filename: String,
    pub document_root: String,
    pub server_protocol: String,
    pub server_name: String,
    pub server_port: u16,
    /// Connection scheme (`http` or `https`) for CGI `REQUEST_SCHEME`/`HTTPS`.
    pub scheme: String,
    pub query_string: Option<String>,
    pub content_type: Option<String>,
    pub path_info: Option<String>,
    pub remote_addr: String,
    pub http_headers: Vec<(String, String)>,
    pub stdin: Vec<u8>,
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
            scheme: "http".to_string(),
            query_string: None,
            content_type: None,
            path_info: None,
            remote_addr: DEFAULT_REMOTE_ADDR.to_string(),
            http_headers: Vec::new(),
            stdin: Vec::new(),
        }
    }

    /// POST with body; sets CONTENT_LENGTH / CONTENT_TYPE when stdin non-empty.
    pub fn post(
        request_uri: &str,
        script_name: &str,
        script_filename: &str,
        content_type: &str,
        body: &[u8],
    ) -> Self {
        let mut req = Self::get(request_uri, script_name, script_filename);
        req.request_method = "POST".to_string();
        req.content_type = Some(content_type.to_string());
        req.stdin = body.to_vec();
        req
    }

    /// Validate required fields and emit PARAMS pairs.
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

        let stdin_len = self.stdin.len();
        if stdin_len > 0 && self.content_type.as_deref().unwrap_or("").is_empty() {
            return Err(ParamsError::BodyWithoutContentLength);
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

        let scheme = if self.scheme.eq_ignore_ascii_case("https") {
            "https"
        } else {
            "http"
        };
        push_param(&mut params, "REQUEST_SCHEME", scheme)?;
        if scheme == "https" {
            push_param(&mut params, "HTTPS", "on")?;
        }

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
        if stdin_len > 0 {
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
            let Some(cgi_name) = http_header_to_cgi_param(name) else {
                continue;
            };
            if seen.contains(&cgi_name) {
                // EXY-11: repeated field — merge into the earlier param instead
                // of dropping it. `Cookie` joins with "; " (RFC 6265 §5.4);
                // other CGI params join with ", " (RFC 3875 / RFC 7230 §3.2.2).
                let merged_sep = if cgi_name == "HTTP_COOKIE" {
                    "; "
                } else {
                    ", "
                };
                if let Some((_, existing)) = params.iter_mut().find(|(k, _)| *k == cgi_name) {
                    let merged = format!("{existing}{merged_sep}{value}");
                    validate_param_pair(&cgi_name, &merged)?;
                    *existing = merged;
                }

                continue;
            }
            push_param(&mut params, &cgi_name, value)?;
            seen.insert(cgi_name);
        }

        // Hyper may expose the authority separately from the header list. CGI
        // clients and WordPress use HTTP_HOST for absolute admin/login URLs;
        // never leave it empty when SERVER_NAME is known.
        if !seen.contains("HTTP_HOST") {
            let host = if self.server_name.is_empty() {
                "localhost"
            } else {
                self.server_name.as_str()
            };
            push_param(&mut params, "HTTP_HOST", host)?;
        }

        Ok(params)
    }
}

/// Connection option tokens that nominate hop-by-hop header field names (RFC 7230 §6.1).
fn connection_nominated_names_from_pairs(headers: &[(String, String)]) -> Vec<String> {
    let mut out = Vec::new();
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("connection") {
            continue;
        }
        for token in exyonq_module_api::connection_nominating_tokens_from_str(value) {
            out.push(token.to_string());
        }
    }
    out
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

/// Map an HTTP header to `HTTP_*` CGI param when safe.
pub fn http_header_to_cgi_param(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if exyonq_module_api::is_fixed_hop_by_hop_header(name) {
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
    fn maps_general_header_to_http_param() {
        assert_eq!(
            http_header_to_cgi_param("X-Custom-Header"),
            Some("HTTP_X_CUSTOM_HEADER".into())
        );
    }

    #[test]
    fn skips_hop_by_hop_and_cgi_specific() {
        assert!(http_header_to_cgi_param("Connection").is_none());
        assert!(http_header_to_cgi_param("Content-Type").is_none());
        assert_eq!(http_header_to_cgi_param("Host"), Some("HTTP_HOST".into()));
    }

    #[test]
    fn skips_every_fixed_hop_member() {
        for &name in exyonq_module_api::FIXED_HOP_BY_HOP_HEADERS {
            assert!(
                http_header_to_cgi_param(name).is_none(),
                "fixed hop {name} must not become CGI HTTP_*"
            );
        }
        assert_eq!(
            http_header_to_cgi_param("Authorization"),
            Some("HTTP_AUTHORIZATION".into())
        );
        assert_eq!(
            http_header_to_cgi_param("WWW-Authenticate"),
            Some("HTTP_WWW_AUTHENTICATE".into())
        );
    }

    #[test]
    fn skips_connection_nominated_custom_headers() {
        let mut req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        req.http_headers = vec![
            ("Connection".into(), "X-Secret".into()),
            ("X-Secret".into(), "nope".into()),
            ("X-End-To-End".into(), "yes".into()),
        ];
        let params = req.to_fcgi_params().expect("params");
        assert!(!params.iter().any(|(k, _)| k == "HTTP_X_SECRET"));
        assert!(params
            .iter()
            .any(|(k, v)| k == "HTTP_X_END_TO_END" && v == "yes"));
    }

    #[test]
    fn merges_repeated_cookie_headers_with_semicolon() {
        // EXY-11: HTTP/2 fragmented Cookie lines must reach PHP as one
        // "; "-joined HTTP_COOKIE — WordPress auth needs every cookie.
        let mut req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        req.http_headers = vec![
            ("Cookie".into(), "a=1".into()),
            ("cookie".into(), "b=2".into()),
            ("Cookie".into(), "c=3".into()),
        ];
        let params = req.to_fcgi_params().expect("params");
        let cookies: Vec<&str> = params
            .iter()
            .filter(|(k, _)| k == "HTTP_COOKIE")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(cookies, ["a=1; b=2; c=3"]);
    }

    #[test]
    fn merges_other_repeated_headers_with_comma() {
        let mut req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        req.http_headers = vec![
            ("Accept".into(), "text/html".into()),
            ("Accept".into(), "application/json".into()),
        ];
        let params = req.to_fcgi_params().expect("params");
        let accepts: Vec<&str> = params
            .iter()
            .filter(|(k, _)| k == "HTTP_ACCEPT")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(accepts, ["text/html, application/json"]);
    }

    #[test]
    fn missing_host_falls_back_to_server_name() {
        let req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        let params = req.to_fcgi_params().expect("params");
        assert!(params
            .iter()
            .any(|(k, v)| k == "HTTP_HOST" && v == "localhost"));
    }

    #[test]
    fn https_scheme_emits_https_and_request_scheme() {
        let mut req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        req.scheme = "https".into();
        let params = req.to_fcgi_params().expect("params");
        assert!(params.iter().any(|(k, v)| k == "HTTPS" && v == "on"));
        assert!(params
            .iter()
            .any(|(k, v)| k == "REQUEST_SCHEME" && v == "https"));
    }

    #[test]
    fn http_scheme_does_not_claim_https() {
        let req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        let params = req.to_fcgi_params().expect("params");
        assert!(!params.iter().any(|(k, _)| k == "HTTPS"));
        assert!(params
            .iter()
            .any(|(k, v)| k == "REQUEST_SCHEME" && v == "http"));
    }

    #[test]
    fn includes_gateway_interface_and_document_root() {
        let req = MinForwardRequest::get("/index.php", "/index.php", "/var/www/index.php");
        let params = req.to_fcgi_params().expect("params");
        assert!(params
            .iter()
            .any(|(k, v)| k == "GATEWAY_INTERFACE" && v == "CGI/1.1"));
        assert!(params.iter().any(|(k, _)| k == "DOCUMENT_ROOT"));
    }
}
