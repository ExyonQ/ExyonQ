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
//! AP-04 — bounded external authorization subrequest (`auth_request`-style).
//!
//! The auth endpoint is an absolute URL. ExyonQ sends a `GET` (no client body),
//! forwards only end-to-end request headers, and fails closed on transport or
//! protocol errors. `2xx` allows the request; `401` challenges and `403` denies;
//! every other status is reported as an unavailable auth decision.

use bytes::Bytes;
use exyonq_config_ir::{AuthConfig, AuthRequestConfig};
use http_body_util::{BodyExt, Empty};
use hyper::header::{HeaderName, HeaderValue};
use hyper::Uri;
use std::time::Duration;

const MAX_FORWARD_HEADERS: usize = 64;
/// Total budget for forwarded header values, not a per-header allowance.
const MAX_FORWARD_HEADER_BYTES: usize = 8 * 1024;
const MAX_AUTH_USER_BYTES: usize = 256;
/// These names carry the trusted target/identity contract across the auth hop.
/// Client-supplied copies must never reach the endpoint: a second value would
/// let the client choose which URI, method, host or user the auth service sees.
const RESERVED_INTERNAL_HEADERS: [&str; 5] = [
    "x-exyonq-original-uri",
    "x-exyonq-original-method",
    "x-exyonq-original-host",
    "x-auth-request-user",
    "x-exyonq-user",
];
/// Bounded cleanup read so a trickling auth server cannot pin the request.
const MAX_DRAIN_TIME: Duration = Duration::from_millis(100);
const MAX_DRAIN_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AuthExternalOutcome {
    Allowed { user: Option<String> },
    Challenge { www_authenticate: Option<String> },
    Forbidden,
    Unavailable,
}

fn is_reserved_internal_header(name: &str) -> bool {
    RESERVED_INTERNAL_HEADERS
        .iter()
        .any(|reserved| name.eq_ignore_ascii_case(reserved))
}

fn is_forwardable(name: &str) -> bool {
    if name.eq_ignore_ascii_case("host")
        || name.eq_ignore_ascii_case("content-length")
        || is_reserved_internal_header(name)
    {
        return false;
    }
    !exyonq_module_api::hop_by_hop::is_fixed_hop_by_hop_header(name)
}

/// Prefer the literal client `Host` header (it keeps the port) over the
/// route-matching host, which the router normalizes to the host name.
fn original_host<'a>(
    headers: &'a [(String, String)],
    fallback: Option<&'a str>,
) -> Option<&'a str> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.as_str())
        .or(fallback)
}

/// Header names nominated by `Connection` are hop-by-hop for this subrequest
/// too: forwarding them would leak a header the client scoped to one hop.
fn connection_nominated(headers: &[(String, String)]) -> Vec<String> {
    let mut names = Vec::new();
    for (name, value) in headers {
        if !name.eq_ignore_ascii_case("connection") {
            continue;
        }
        for token in value.split(',') {
            let token = token.trim();
            if token.is_empty()
                || token.eq_ignore_ascii_case("close")
                || token.eq_ignore_ascii_case("keep-alive")
            {
                continue;
            }
            names.push(token.to_ascii_lowercase());
        }
    }
    names
}

fn sanitized_user(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.len() > MAX_AUTH_USER_BYTES {
        return None;
    }
    if value.chars().any(|ch| ch.is_control()) {
        return None;
    }
    Some(value.to_string())
}

fn classify(
    status: u16,
    www_authenticate: Option<&str>,
    user: Option<&str>,
) -> AuthExternalOutcome {
    match status {
        200..=299 => AuthExternalOutcome::Allowed {
            user: sanitized_user(user),
        },
        401 => AuthExternalOutcome::Challenge {
            www_authenticate: www_authenticate.map(str::to_string),
        },
        403 => AuthExternalOutcome::Forbidden,
        _ => AuthExternalOutcome::Unavailable,
    }
}

/// Return the configured external-auth decision, or `None` when this route uses
/// basic auth / no auth. Errors never allow the request.
pub(crate) async fn check(
    config: &AuthConfig,
    route_name: &str,
    method: &hyper::Method,
    uri: &Uri,
    host: Option<&str>,
    headers: &[(String, String)],
) -> Option<AuthExternalOutcome> {
    let route = config.routes.get(route_name)?;
    let request = route.auth_request.as_ref()?;
    Some(check_request(request, method, uri, host, headers).await)
}

async fn check_request(
    config: &AuthRequestConfig,
    method: &hyper::Method,
    uri: &Uri,
    host: Option<&str>,
    headers: &[(String, String)],
) -> AuthExternalOutcome {
    let endpoint: Uri = match config.endpoint.parse() {
        Ok(uri) => uri,
        Err(_) => return AuthExternalOutcome::Unavailable,
    };
    let mut builder = hyper::Request::builder()
        .method(hyper::Method::GET)
        .uri(endpoint);
    let original_uri = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    builder = builder.header("x-exyonq-original-uri", original_uri);
    builder = builder.header("x-exyonq-original-method", method.as_str());
    if let Some(host) =
        original_host(headers, host).and_then(|value| HeaderValue::from_str(value).ok())
    {
        builder = builder.header("x-exyonq-original-host", host);
    }
    let nominated = connection_nominated(headers);
    let mut forwarded = 0usize;
    let mut forwarded_bytes = 0usize;
    for (name, value) in headers {
        if forwarded >= MAX_FORWARD_HEADERS {
            break;
        }
        if !is_forwardable(name)
            || nominated
                .iter()
                .any(|nominated| nominated.eq_ignore_ascii_case(name))
        {
            continue;
        }
        let Some(budget_left) = MAX_FORWARD_HEADER_BYTES.checked_sub(forwarded_bytes) else {
            break;
        };
        if value.len() > budget_left {
            continue;
        }
        let Ok(name) = HeaderName::from_bytes(name.as_bytes()) else {
            continue;
        };
        let Ok(value) = HeaderValue::from_str(value) else {
            continue;
        };
        forwarded_bytes += value.as_bytes().len();
        builder = builder.header(name, value);
        forwarded += 1;
    }
    let request = match builder.body(Empty::<Bytes>::new()) {
        Ok(request) => request,
        Err(_) => return AuthExternalOutcome::Unavailable,
    };
    let client = exyonq_mod_proxy::get_empty_body_client();
    let response = match tokio::time::timeout(
        Duration::from_millis(u64::from(config.timeout_ms)),
        client.request(request),
    )
    .await
    {
        Ok(Ok(response)) => response,
        _ => return AuthExternalOutcome::Unavailable,
    };
    let status = response.status().as_u16();
    let www_authenticate = response
        .headers()
        .get(hyper::header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let user = response
        .headers()
        .get("x-auth-request-user")
        .or_else(|| response.headers().get("x-exyonq-user"))
        .and_then(|value| value.to_str().ok());
    let outcome = classify(status, www_authenticate.as_deref(), user);
    // Drain a bounded amount so the upstream connection can be reused, but
    // never let an auth server send an unbounded body or trickle forever into
    // this path: the whole cleanup read shares one short deadline.
    let mut body = response.into_body();
    let mut consumed = 0usize;
    let deadline = tokio::time::Instant::now() + MAX_DRAIN_TIME;
    while consumed < MAX_DRAIN_BYTES {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, body.frame()).await {
            Ok(Some(Ok(frame))) => consumed += frame.data_ref().map_or(0, Bytes::len),
            _ => break,
        }
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_is_fail_closed() {
        assert_eq!(
            classify(204, None, Some("alice")),
            AuthExternalOutcome::Allowed {
                user: Some("alice".into())
            }
        );
        assert_eq!(
            classify(401, Some("Bearer realm=api"), None),
            AuthExternalOutcome::Challenge {
                www_authenticate: Some("Bearer realm=api".into())
            }
        );
        assert_eq!(classify(403, None, None), AuthExternalOutcome::Forbidden);
        assert_eq!(classify(302, None, None), AuthExternalOutcome::Unavailable);
        assert_eq!(classify(500, None, None), AuthExternalOutcome::Unavailable);
    }

    #[test]
    fn user_header_is_bounded_and_control_free() {
        assert_eq!(sanitized_user(Some(" alice ")), Some("alice".into()));
        assert_eq!(sanitized_user(Some("a\nbb")), None);
        assert_eq!(sanitized_user(Some(&"x".repeat(257))), None);
    }

    #[test]
    fn forwarded_headers_keep_end_to_end_auth_and_drop_hop_local_fields() {
        for name in [
            "authorization",
            "cookie",
            "x-request-id",
            "content-type",
            "user-agent",
        ] {
            assert!(is_forwardable(name), "{name} must reach the auth endpoint");
        }
        for name in [
            "host",
            "content-length",
            "connection",
            "keep-alive",
            "proxy-authorization",
            "proxy-connection",
            "te",
            "trailer",
            "transfer-encoding",
            "upgrade",
            "x-exyonq-original-uri",
            "x-exyonq-original-method",
            "x-exyonq-original-host",
            "x-auth-request-user",
            "x-exyonq-user",
        ] {
            assert!(!is_forwardable(name), "{name} crossed the auth hop");
        }
    }

    #[test]
    fn original_host_prefers_the_literal_host_header() {
        let headers = vec![("Host".to_string(), "api.example:8443".to_string())];
        assert_eq!(
            original_host(&headers, Some("api.example")),
            Some("api.example:8443")
        );
        assert_eq!(original_host(&[], Some("api.example")), Some("api.example"));
        assert_eq!(original_host(&[], None), None);
    }

    #[test]
    fn connection_nominated_headers_are_not_forwarded() {
        let headers = vec![
            ("connection".to_string(), "close, X-Hop".to_string()),
            ("x-hop".to_string(), "secret".to_string()),
            ("authorization".to_string(), "Bearer token".to_string()),
        ];
        let nominated = connection_nominated(&headers);
        assert_eq!(nominated, vec!["x-hop".to_string()]);
        assert!(nominated.iter().any(|name| name == "x-hop"));
        assert!(is_forwardable("authorization"));
    }
}
