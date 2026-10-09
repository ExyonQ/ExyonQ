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
//! Wire path eligibility predicates (KD2.3 / KD2.5).
//!
//! Cap061: MUST NOT key eligibility off benchmark filenames (bench-sized assets / route tables).
//! Cap067: automatic Linux cleartext sendfile eligibility from real product properties
//! (method, origin-form path, mechanism auto-enable). Asset resolve happens in `match_sendfile_asset`.
//!
//! HTTP/1.1 request-target may be origin-form (`/path`) or absolute-form
//! (`http://host[:port]/path`, RFC 9112 §3.2). Eligibility and asset match use
//! the origin path (same as Hyper `Uri::path()`), never the raw target. This is
//! production HTTP/1.1 correctness, not a loadgen or filename special case.

/// Paths that stay on Cap067 inline wire / sendfile divert (not Hyper on accept).
///
/// `/health` is the only inline ops probe. `/metrics` is **not** wire-served
/// (LA-CAP054-008: OpenMetrics lives on Hyper + `[modules.metrics]`); Cap067
/// must Hyper-handoff it at accept so EPOLL listen does not idle-hang.
pub fn might_use_static_wire(head: &[u8]) -> bool {
    if is_inline_ops_probe_head(head) {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        // Cap067: admit GET/HEAD candidates so WAF → sendfile divert can run;
        // ineligible resolve falls back to Hyper (never filename allowlists).
        epoll_sendfile_eligible(head)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// `/health` only — Cap067 inline wire has a fixed keep-alive response.
fn is_inline_ops_probe_head(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ") || head.starts_with(b"HEAD /health ")
}

#[cfg(target_os = "linux")]
pub fn epoll_keep_alive_eligible(head: &[u8]) -> bool {
    head.starts_with(b"GET /health ")
}

/// Head-level sendfile candidate (Cap067). Resolve/open happens in `match_sendfile_asset`.
///
/// Product properties only: Linux mechanism auto-on, GET/HEAD, origin-form path
/// (origin-form or http(s) absolute-form request-target), not `/health` inline
/// wire or `/metrics` Hyper scrape, no Upgrade: websocket.
#[cfg(target_os = "linux")]
pub fn epoll_sendfile_eligible(head: &[u8]) -> bool {
    if !crate::sendfile_fsm::epoll_sendfile_enabled() {
        return false;
    }
    // OTel request spans are entered on the Hyper path.
    if exyonq_module_api::static_wire::wire_otel_spans_enabled() {
        return false;
    }
    if is_inline_ops_probe_head(head) {
        return false;
    }
    if head_has_websocket_upgrade(head) {
        return false;
    }
    let Some((method, path)) = method_and_path(head) else {
        return false;
    };
    if method != b"GET" && method != b"HEAD" {
        return false;
    }
    if !path.starts_with(b"/") {
        return false;
    }
    // Origin-form extract of `/health` / `/metrics` (including absolute-form).
    // `/metrics` excluded here even though it is not an inline probe — Hyper only.
    if path == b"/health" || path == b"/live" || path == b"/ready" || path == b"/metrics" {
        return false;
    }
    // Product surface: proxy API is never a static sendfile candidate.
    if path == b"/api" || path.starts_with(b"/api/") {
        return false;
    }
    true
}

#[cfg(target_os = "linux")]
pub fn static_wire_use_blocking_pool(_head: &[u8]) -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("EXYONQ_STATIC_BLOCKING").ok().as_deref() == Some("1"))
}

#[cfg(target_os = "linux")]
pub fn static_sendfile_use_blocking_pool(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_keep_alive_eligible(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn epoll_sendfile_eligible(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn static_wire_use_blocking_pool(_head: &[u8]) -> bool {
    false
}

#[cfg(not(target_os = "linux"))]
pub fn static_sendfile_use_blocking_pool(_head: &[u8]) -> bool {
    false
}

/// Origin-form path from an HTTP/1.1 request-target (RFC 9112).
///
/// - origin-form `/path[?query]` → `/path`
/// - absolute-form `http(s)://authority[/path][?query]` → `/path` (authority-only → `/`)
/// - asterisk-form / authority-form / unknown schemes → `None`
#[cfg(any(test, target_os = "linux"))]
fn origin_path_from_request_target(target: &[u8]) -> Option<&[u8]> {
    if target.is_empty() || target == b"*" {
        return None;
    }
    let path_and_query = if target[0] == b'/' {
        target
    } else {
        let rest = strip_http_absolute_scheme(target)?;
        match rest.iter().position(|&b| b == b'/') {
            Some(i) => &rest[i..],
            None => b"/",
        }
    };
    let end = path_and_query
        .iter()
        .position(|&b| b == b'?' || b == b'#')
        .unwrap_or(path_and_query.len());
    let path = &path_and_query[..end];
    if path.is_empty() || path[0] != b'/' {
        return None;
    }
    Some(path)
}

#[cfg(any(test, target_os = "linux"))]
fn strip_http_absolute_scheme(target: &[u8]) -> Option<&[u8]> {
    if target.len() >= 8 && target[..8].eq_ignore_ascii_case(b"https://") {
        return Some(&target[8..]);
    }
    if target.len() >= 7 && target[..7].eq_ignore_ascii_case(b"http://") {
        return Some(&target[7..]);
    }
    None
}

#[cfg(target_os = "linux")]
fn method_and_path(head: &[u8]) -> Option<(&[u8], &[u8])> {
    let line_end = head
        .iter()
        .position(|&b| b == b'\r' || b == b'\n')
        .unwrap_or(head.len());
    let line = &head[..line_end];
    let mut parts = line.split(|&b| b == b' ');
    let method = parts.next()?;
    let target = parts.next()?;
    let path = origin_path_from_request_target(target)?;
    Some((method, path))
}

#[cfg(target_os = "linux")]
fn head_has_websocket_upgrade(head: &[u8]) -> bool {
    let lower = |b: u8| b.to_ascii_lowercase();
    // Cheap scan: Upgrade: websocket (case-insensitive field value token).
    let mut i = 0usize;
    while i + 9 < head.len() {
        if lower(head[i]) == b'u'
            && lower(head[i + 1]) == b'p'
            && lower(head[i + 2]) == b'g'
            && lower(head[i + 3]) == b'r'
            && lower(head[i + 4]) == b'a'
            && lower(head[i + 5]) == b'd'
            && lower(head[i + 6]) == b'e'
            && head[i + 7] == b':'
        {
            let rest = &head[i + 8..];
            let end = rest
                .iter()
                .position(|&b| b == b'\r' || b == b'\n')
                .unwrap_or(rest.len());
            let val = &rest[..end];
            if val.windows(9).any(|w| {
                w.iter()
                    .map(|b| b.to_ascii_lowercase())
                    .eq(b"websocket".iter().copied())
            }) {
                return true;
            }
        }
        i += 1;
    }
    false
}

/// Parse origin-form request path from a raw HTTP/1 request head (no query).
#[cfg(target_os = "linux")]
pub fn request_path_from_head(head: &[u8]) -> Option<&str> {
    let (_, path) = method_and_path(head)?;
    std::str::from_utf8(path).ok()
}

#[cfg(test)]
mod origin_path_tests {
    use super::origin_path_from_request_target;

    #[test]
    fn origin_form_path_strips_query() {
        assert_eq!(
            origin_path_from_request_target(b"/site/1k.bin?x=1"),
            Some(b"/site/1k.bin".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"/site/1k.bin"),
            Some(b"/site/1k.bin".as_slice())
        );
    }

    #[test]
    fn absolute_form_http_extracts_origin_path() {
        assert_eq!(
            origin_path_from_request_target(b"http://exyonq:8080/site/1k.bin"),
            Some(b"/site/1k.bin".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"HTTP://exyonq:8080/site/1k.bin?cache=0"),
            Some(b"/site/1k.bin".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"https://host/site/1k.bin"),
            Some(b"/site/1k.bin".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"http://[::1]:8080/site/1k.bin"),
            Some(b"/site/1k.bin".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"http://exyonq:8080"),
            Some(b"/".as_slice())
        );
    }

    #[test]
    fn absolute_form_api_and_ops_keep_origin_path() {
        assert_eq!(
            origin_path_from_request_target(b"http://exyonq:8080/api/foo"),
            Some(b"/api/foo".as_slice())
        );
        assert_eq!(
            origin_path_from_request_target(b"http://exyonq:8080/health"),
            Some(b"/health".as_slice())
        );
    }

    #[test]
    fn asterisk_and_authority_form_are_not_static_paths() {
        assert_eq!(origin_path_from_request_target(b"*"), None);
        assert_eq!(origin_path_from_request_target(b"example.com:443"), None);
        assert_eq!(origin_path_from_request_target(b""), None);
    }
}
