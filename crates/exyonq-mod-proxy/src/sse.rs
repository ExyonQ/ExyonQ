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
//! SSE response classification.
//!
//! L-02: the upstream timeout is the timeout compiled for the upstream
//! (`[[upstream]] timeout_ms`), independent of the request path. There is no
//! benchmark path exception.

/// True when `Content-Type` indicates an event stream response.
pub fn content_type_is_event_stream(content_type: Option<&str>) -> bool {
    content_type.is_some_and(|ct| {
        ct.split(';')
            .next()
            .is_some_and(|mime| mime.eq_ignore_ascii_case("text/event-stream"))
    })
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
}
