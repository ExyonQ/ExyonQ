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
//! Explicit response observation for `Module::on_response` (Cap032 / Cap054).
//!
//! Distinguishes a genuinely empty finite body from a streaming body that must
//! not be collected (SSE). Collapsing those into `Body::empty()` is forbidden.

use bytes::Bytes;
use http::{HeaderMap, Response, StatusCode};

use crate::{Body, HttpResponse};

/// How the response body is available to post-response modules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResponseBodyState {
    /// Finite body bytes available for observation and optional transform.
    Available(Bytes),
    /// Finite response whose body is genuinely empty.
    Empty,
    /// Streaming body (e.g. `text/event-stream`) — not collected, not empty product data.
    StreamingUnavailable,
}

/// Status + headers + explicit body availability for `Module::on_response`.
#[derive(Debug, Clone)]
pub struct ResponseObservation {
    status: StatusCode,
    headers: HeaderMap,
    body: ResponseBodyState,
}

impl ResponseObservation {
    pub fn streaming_unavailable(status: StatusCode, headers: HeaderMap) -> Self {
        Self {
            status,
            headers,
            body: ResponseBodyState::StreamingUnavailable,
        }
    }

    /// Build from a finite module `HttpResponse` (`Full` body).
    ///
    /// Precondition: `body` must not have been polled. `http_body_util::Full`
    /// stores a genuine empty payload as `into_inner() == None`, which maps to
    /// [`ResponseBodyState::Empty`] — not to streaming.
    pub fn from_http_response(resp: HttpResponse) -> Self {
        let (parts, body) = resp.into_parts();
        // Full encodes empty bytes as None; Some(empty) is not used by Full::new.
        let bytes = body.into_inner().unwrap_or_default();
        let body = if bytes.is_empty() {
            ResponseBodyState::Empty
        } else {
            ResponseBodyState::Available(bytes)
        };
        Self {
            status: parts.status,
            headers: parts.headers,
            body,
        }
    }

    /// Materialize a finite observation back into an `HttpResponse`.
    ///
    /// Fails for [`ResponseBodyState::StreamingUnavailable`] — streaming bodies
    /// must remain on the original hyper response stream.
    pub fn try_into_http_response(self) -> Result<HttpResponse, StreamingBodyError> {
        let bytes = match self.body {
            ResponseBodyState::Available(b) => b,
            ResponseBodyState::Empty => Bytes::new(),
            ResponseBodyState::StreamingUnavailable => {
                return Err(StreamingBodyError);
            }
        };
        let mut resp = Response::new(Body::from(bytes));
        *resp.status_mut() = self.status;
        *resp.headers_mut() = self.headers;
        Ok(resp)
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn status_mut(&mut self) -> &mut StatusCode {
        &mut self.status
    }

    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    pub fn headers_mut(&mut self) -> &mut HeaderMap {
        &mut self.headers
    }

    pub fn body(&self) -> &ResponseBodyState {
        &self.body
    }

    /// Replace finite body bytes. Refuses to overwrite [`ResponseBodyState::StreamingUnavailable`].
    pub fn set_body_bytes(&mut self, bytes: Bytes) -> Result<(), StreamingBodyError> {
        if self.is_streaming_unavailable() {
            return Err(StreamingBodyError);
        }
        self.body = if bytes.is_empty() {
            ResponseBodyState::Empty
        } else {
            ResponseBodyState::Available(bytes)
        };
        Ok(())
    }

    /// Clear a finite body to Empty. Refuses streaming → empty collapse.
    pub fn clear_body(&mut self) -> Result<(), StreamingBodyError> {
        if self.is_streaming_unavailable() {
            return Err(StreamingBodyError);
        }
        self.body = ResponseBodyState::Empty;
        Ok(())
    }

    /// Take finite body bytes when available (`Available` or `Empty`).
    ///
    /// Returns `None` for [`ResponseBodyState::StreamingUnavailable`] without
    /// changing that state — callers must not invent an empty product body.
    pub fn take_finite_bytes(&mut self) -> Option<Bytes> {
        match std::mem::replace(&mut self.body, ResponseBodyState::Empty) {
            ResponseBodyState::Available(b) => Some(b),
            ResponseBodyState::Empty => Some(Bytes::new()),
            ResponseBodyState::StreamingUnavailable => {
                self.body = ResponseBodyState::StreamingUnavailable;
                None
            }
        }
    }

    pub fn is_streaming_unavailable(&self) -> bool {
        matches!(self.body, ResponseBodyState::StreamingUnavailable)
    }
}

/// Attempted to materialize or mutate a streaming-unavailable observation body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamingBodyError;

impl std::fmt::Display for StreamingBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cannot treat StreamingUnavailable as a finite response body")
    }
}

impl std::error::Error for StreamingBodyError {}

#[cfg(test)]
mod tests {
    use super::*;
    use http::header::{CACHE_CONTROL, CONTENT_TYPE};

    #[test]
    fn empty_bytes_are_empty_state_not_streaming() {
        let resp = Response::builder()
            .status(StatusCode::NO_CONTENT)
            .body(Body::from(Bytes::new()))
            .unwrap();
        let obs = ResponseObservation::from_http_response(resp);
        assert_eq!(obs.status(), StatusCode::NO_CONTENT);
        assert_eq!(obs.body(), &ResponseBodyState::Empty);
        assert!(!obs.is_streaming_unavailable());
    }

    #[test]
    fn streaming_unavailable_preserves_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, "text/event-stream".parse().unwrap());
        headers.insert(CACHE_CONTROL, "no-cache".parse().unwrap());
        headers.insert("x-trace", "abc".parse().unwrap());
        let obs = ResponseObservation::streaming_unavailable(StatusCode::OK, headers.clone());
        assert!(obs.is_streaming_unavailable());
        assert_eq!(
            obs.headers().get(CONTENT_TYPE).unwrap(),
            "text/event-stream"
        );
        assert_eq!(obs.headers().get(CACHE_CONTROL).unwrap(), "no-cache");
        assert_eq!(obs.headers().get("x-trace").unwrap(), "abc");
        assert!(obs.try_into_http_response().is_err());
    }

    #[test]
    fn take_finite_bytes_does_not_collapse_streaming() {
        let mut obs = ResponseObservation::streaming_unavailable(StatusCode::OK, HeaderMap::new());
        assert!(obs.take_finite_bytes().is_none());
        assert!(obs.is_streaming_unavailable());
    }

    #[test]
    fn mutators_refuse_to_collapse_streaming_to_empty() {
        let mut obs = ResponseObservation::streaming_unavailable(StatusCode::OK, HeaderMap::new());
        assert!(obs.clear_body().is_err());
        assert!(obs.is_streaming_unavailable());
        assert!(obs.set_body_bytes(Bytes::new()).is_err());
        assert!(obs.is_streaming_unavailable());
        assert!(obs.set_body_bytes(Bytes::from_static(b"x")).is_err());
        assert!(obs.is_streaming_unavailable());
    }
}
