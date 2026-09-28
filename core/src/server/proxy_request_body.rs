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
//! Bounded proxy request-body materialization (SEC-PROXY-001).
//!
//! Defense in depth:
//! 1. Optional early reject when a valid Content-Length exceeds the limit
//!    (optimization only — Content-Length is not authoritative).
//! 2. Incremental frame accumulation with checked arithmetic; abort before
//!    appending a chunk that would exceed the limit.

use http_body_util::BodyExt;
use hyper::body::Body;
use hyper::header::{HeaderMap, CONTENT_LENGTH};

/// Private error for proxy request body reads (not a public API).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProxyRequestBodyReadError {
    /// Transport / body frame error. Caller must fail closed (no empty-body fallback).
    Body,
    /// Declared or observed size exceeds the configured limit.
    TooLarge,
}

/// Parse a single valid Content-Length as `usize`.
///
/// Invalid or absent values yield `None` (incremental enforcement only).
pub(crate) fn parse_proxy_request_content_length(headers: &HeaderMap) -> Option<usize> {
    let value = headers.get(CONTENT_LENGTH)?;
    let raw = value.to_str().ok()?;
    raw.parse::<usize>().ok()
}

/// Whether appending `chunk_len` bytes would stay within `max_bytes`.
///
/// Returns the new total on success. `None` means overflow or over-limit
/// (caller must not append).
pub(crate) fn proxy_request_body_accept_chunk(
    current_len: usize,
    chunk_len: usize,
    max_bytes: usize,
) -> Option<usize> {
    current_len
        .checked_add(chunk_len)
        .filter(|total| *total <= max_bytes)
}

/// Collect a proxy request body with an authoritative byte limit.
///
/// On [`ProxyRequestBodyReadError::TooLarge`], stops reading further data frames
/// and drops the partial buffer. Does not contact upstream (caller must not).
pub(crate) async fn collect_proxy_request_body_bounded<B>(
    declared_content_length: Option<usize>,
    mut body: B,
    max_bytes: usize,
) -> Result<Vec<u8>, ProxyRequestBodyReadError>
where
    B: Body + Unpin,
    B::Data: AsRef<[u8]>,
{
    if declared_content_length.is_some_and(|len| len > max_bytes) {
        return Err(ProxyRequestBodyReadError::TooLarge);
    }

    let mut buf = Vec::new();
    loop {
        match body.frame().await {
            None => return Ok(buf),
            Some(Err(_)) => return Err(ProxyRequestBodyReadError::Body),
            Some(Ok(frame)) => {
                let Ok(chunk) = frame.into_data() else {
                    // Trailers / non-data frames: ignore (existing Hyper semantics).
                    continue;
                };
                let chunk = chunk.as_ref();
                if chunk.is_empty() {
                    continue;
                }
                let Some(_total) =
                    proxy_request_body_accept_chunk(buf.len(), chunk.len(), max_bytes)
                else {
                    drop(buf);
                    return Err(ProxyRequestBodyReadError::TooLarge);
                };
                buf.extend_from_slice(chunk);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use http_body_util::BodyExt;
    use http_body_util::Full;
    use hyper::body::{Body, Frame};
    use hyper::header::HeaderValue;
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// Multi-frame body for deterministic frame-boundary tests (no extra deps).
    struct ChunksBody {
        chunks: VecDeque<Bytes>,
        trailers: bool,
        emit_trailer: bool,
    }

    impl ChunksBody {
        fn data(chunks: impl IntoIterator<Item = Bytes>) -> Self {
            Self {
                chunks: chunks.into_iter().collect(),
                trailers: false,
                emit_trailer: false,
            }
        }

        fn with_trailer(mut self) -> Self {
            self.trailers = true;
            self
        }
    }

    impl Body for ChunksBody {
        type Data = Bytes;
        type Error = std::convert::Infallible;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            if let Some(chunk) = self.chunks.pop_front() {
                return Poll::Ready(Some(Ok(Frame::data(chunk))));
            }
            if self.trailers && !self.emit_trailer {
                self.emit_trailer = true;
                let mut map = HeaderMap::new();
                map.insert("x-trailer", HeaderValue::from_static("1"));
                return Poll::Ready(Some(Ok(Frame::trailers(map))));
            }
            Poll::Ready(None)
        }
    }

    struct ErrorBody;

    impl Body for ErrorBody {
        type Data = Bytes;
        type Error = std::io::Error;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            Poll::Ready(Some(Err(std::io::Error::other("body read failed"))))
        }
    }

    #[test]
    fn accept_chunk_boundary_and_overflow() {
        assert_eq!(proxy_request_body_accept_chunk(0, 0, 10), Some(0));
        assert_eq!(proxy_request_body_accept_chunk(9, 1, 10), Some(10));
        assert_eq!(proxy_request_body_accept_chunk(10, 1, 10), None);
        assert_eq!(proxy_request_body_accept_chunk(0, 11, 10), None);
        assert_eq!(
            proxy_request_body_accept_chunk(usize::MAX - 1, 2, usize::MAX),
            None
        );
        assert_eq!(
            proxy_request_body_accept_chunk(usize::MAX - 1, 1, usize::MAX),
            Some(usize::MAX)
        );
    }

    #[tokio::test]
    async fn empty_body_accepted() {
        let body = Full::<Bytes>::new(Bytes::new());
        let out = collect_proxy_request_body_bounded(None, body, 32)
            .await
            .expect("ok");
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn max_minus_one_and_max_accepted() {
        const MAX: usize = 64;
        let almost = vec![b'a'; MAX - 1];
        let exact = vec![b'b'; MAX];
        assert_eq!(
            collect_proxy_request_body_bounded(None, Full::new(Bytes::from(almost.clone())), MAX)
                .await
                .unwrap(),
            almost
        );
        assert_eq!(
            collect_proxy_request_body_bounded(None, Full::new(Bytes::from(exact.clone())), MAX)
                .await
                .unwrap(),
            exact
        );
    }

    #[tokio::test]
    async fn max_plus_one_rejected_before_append() {
        const MAX: usize = 64;
        let body = Full::new(Bytes::from(vec![b'x'; MAX + 1]));
        let err = collect_proxy_request_body_bounded(None, body, MAX)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn many_small_frames_enforce_limit() {
        const MAX: usize = 10;
        let chunks = (0..12).map(|_| Bytes::from_static(b"a"));
        let err = collect_proxy_request_body_bounded(None, ChunksBody::data(chunks), MAX)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn final_frame_crossing_limit_rejected() {
        const MAX: usize = 10;
        let body = ChunksBody::data([Bytes::from(vec![b'a'; 9]), Bytes::from_static(b"xy")]);
        let err = collect_proxy_request_body_bounded(None, body, MAX)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn content_length_over_max_early_reject() {
        const MAX: usize = 32;
        let body = Full::new(Bytes::from_static(b"never-read"));
        let err = collect_proxy_request_body_bounded(Some(MAX + 1), body, MAX)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn content_length_eq_max_still_validated_incrementally() {
        const MAX: usize = 8;
        let ok = collect_proxy_request_body_bounded(
            Some(MAX),
            Full::new(Bytes::from(vec![b'z'; MAX])),
            MAX,
        )
        .await
        .unwrap();
        assert_eq!(ok.len(), MAX);
        let err = collect_proxy_request_body_bounded(
            Some(MAX),
            Full::new(Bytes::from(vec![b'z'; MAX + 1])),
            MAX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn false_small_content_length_still_bounded_by_bytes() {
        const MAX: usize = 8;
        let err = collect_proxy_request_body_bounded(
            Some(2),
            Full::new(Bytes::from(vec![b'x'; MAX + 1])),
            MAX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn missing_content_length_incremental_reject() {
        const MAX: usize = 4;
        let err = collect_proxy_request_body_bounded(
            None,
            Full::new(Bytes::from(vec![0u8; MAX + 1])),
            MAX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn invalid_content_length_falls_back_to_incremental() {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_LENGTH, HeaderValue::from_static("not-a-number"));
        assert_eq!(parse_proxy_request_content_length(&headers), None);
        const MAX: usize = 4;
        let declared = parse_proxy_request_content_length(&headers);
        let ok = collect_proxy_request_body_bounded(
            declared,
            Full::new(Bytes::from_static(b"abcd")),
            MAX,
        )
        .await
        .unwrap();
        assert_eq!(ok, b"abcd");
        let err = collect_proxy_request_body_bounded(
            declared,
            Full::new(Bytes::from_static(b"abcde")),
            MAX,
        )
        .await
        .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn trailers_do_not_bypass_limit() {
        const MAX: usize = 4;
        let body = ChunksBody::data([Bytes::from_static(b"12345")]).with_trailer();
        let err = collect_proxy_request_body_bounded(None, body, MAX)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::TooLarge);
    }

    #[tokio::test]
    async fn trailers_after_valid_body_ok() {
        const MAX: usize = 4;
        let body = ChunksBody::data([Bytes::from_static(b"1234")]).with_trailer();
        let out = collect_proxy_request_body_bounded(None, body, MAX)
            .await
            .unwrap();
        assert_eq!(out, b"1234");
    }

    #[tokio::test]
    async fn body_error_maps_to_body_variant() {
        let body = ErrorBody.map_err(|_| ());
        let err = collect_proxy_request_body_bounded(None, body, 32)
            .await
            .unwrap_err();
        assert_eq!(err, ProxyRequestBodyReadError::Body);
    }

    #[tokio::test]
    async fn chunked_equivalent_many_frames_within_limit() {
        const MAX: usize = 6;
        let body = ChunksBody::data([
            Bytes::from_static(b"ab"),
            Bytes::from_static(b"cd"),
            Bytes::from_static(b"ef"),
        ]);
        let out = collect_proxy_request_body_bounded(None, body, MAX)
            .await
            .unwrap();
        assert_eq!(out, b"abcdef");
    }
}
