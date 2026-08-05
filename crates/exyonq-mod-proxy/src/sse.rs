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
//! SSE path/timeout policy helpers (no streaming FSM in KD3.1).

use std::time::Duration;

/// Bench paths using long upstream timeout (P8).
pub const SSE_STREAM_PATHS: &[&str] = &["/api/stream", "/api/stream-long"];

/// Long-lived SSE streams use a separate upstream timeout (P8).
pub const SSE_STREAM_TIMEOUT: Duration = Duration::from_secs(300);

/// True when `Content-Type` indicates an event stream response.
pub fn content_type_is_event_stream(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| {
        ct.split(';')
            .next()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("text/event-stream"))
    })
}

/// True when request path is a configured SSE bench stream.
pub fn path_is_sse_stream(path_and_query: &str) -> bool {
    let path = path_and_query.split('?').next().unwrap_or(path_and_query);
    SSE_STREAM_PATHS.contains(&path)
}

/// Upstream timeout for a path given cluster default timeout.
pub fn upstream_timeout_for_path(default_timeout: Duration, path_and_query: &str) -> Duration {
    if path_is_sse_stream(path_and_query) {
        SSE_STREAM_TIMEOUT
    } else {
        default_timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_event_stream_mime() {
        assert!(content_type_is_event_stream(Some(
            "text/event-stream; charset=utf-8"
        )));
        assert!(!content_type_is_event_stream(Some("text/plain")));
    }

    #[test]
    fn sse_path_extends_timeout() {
        let normal = Duration::from_millis(5_000);
        assert_eq!(
            upstream_timeout_for_path(normal, "/api/stream"),
            SSE_STREAM_TIMEOUT
        );
        assert_eq!(
            upstream_timeout_for_path(normal, "/api/stream-long"),
            SSE_STREAM_TIMEOUT
        );
        assert_eq!(upstream_timeout_for_path(normal, "/api/health"), normal);
    }
}
