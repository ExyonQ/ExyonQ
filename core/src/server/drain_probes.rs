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
//! Drain-boundary HTTP probe responses (P15-WS5-PROBE-002).
//!
//! When `try_enter` / `try_admit` rejects a new connection during drain, the accept
//! path must still answer liveness probes without admitting the connection into the
//! active set. Readiness fails closed; other paths stay `503 draining`.

/// Generic drain reject (non-probe traffic).
pub(crate) const DRAINING_RESPONSE: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 8\r\nConnection: close\r\n\r\ndraining";

const LIVE_RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 4\r\nConnection: close\r\n\r\nlive";

const HEALTH_RESPONSE: &[u8] =
    b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";

const NOT_READY_RESPONSE: &[u8] =
    b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot_ready";

/// Classify a partially-read HTTP/1 request head into a drain-boundary response.
pub(crate) fn drain_boundary_response(request_head: &[u8]) -> &'static [u8] {
    let line_end = request_head
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(request_head.len());
    let line = &request_head[..line_end];
    // Tolerant of `\r` and absolute-form targets.
    if method_path_is(line, b"/live") {
        return LIVE_RESPONSE;
    }
    if method_path_is(line, b"/health") {
        return HEALTH_RESPONSE;
    }
    if method_path_is(line, b"/ready") {
        return NOT_READY_RESPONSE;
    }
    DRAINING_RESPONSE
}

fn method_path_is(request_line: &[u8], path: &[u8]) -> bool {
    let line = trim_cr(request_line);
    let mut parts = line.split(|&b| b == b' ');
    let method = parts.next().unwrap_or(b"");
    let target = parts.next().unwrap_or(b"");
    if method != b"GET" && method != b"HEAD" {
        return false;
    }
    let path_part = if let Some(rest) = target.strip_prefix(b"http://") {
        rest.iter()
            .position(|&b| b == b'/')
            .map(|i| &rest[i..])
            .unwrap_or(b"/")
    } else if let Some(rest) = target.strip_prefix(b"https://") {
        rest.iter()
            .position(|&b| b == b'/')
            .map(|i| &rest[i..])
            .unwrap_or(b"/")
    } else {
        target
    };
    let path_only = path_part
        .iter()
        .position(|&b| b == b'?' || b == b'#')
        .map(|i| &path_part[..i])
        .unwrap_or(path_part);
    path_only == path
}

fn trim_cr(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r").unwrap_or(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_and_health_pass_during_drain_boundary() {
        assert_eq!(
            drain_boundary_response(b"GET /live HTTP/1.1\r\n"),
            LIVE_RESPONSE
        );
        assert_eq!(
            drain_boundary_response(b"GET /health HTTP/1.1\r\n"),
            HEALTH_RESPONSE
        );
        assert_eq!(
            drain_boundary_response(b"HEAD /live HTTP/1.1\r\n"),
            LIVE_RESPONSE
        );
    }

    #[test]
    fn ready_is_not_ready_during_drain_boundary() {
        assert_eq!(
            drain_boundary_response(b"GET /ready HTTP/1.1\r\n"),
            NOT_READY_RESPONSE
        );
    }

    #[test]
    fn other_paths_stay_draining() {
        assert_eq!(
            drain_boundary_response(b"GET / HTTP/1.1\r\n"),
            DRAINING_RESPONSE
        );
        assert_eq!(
            drain_boundary_response(b"GET /site/x HTTP/1.1\r\n"),
            DRAINING_RESPONSE
        );
    }
}
