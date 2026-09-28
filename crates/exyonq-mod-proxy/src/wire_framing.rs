//! Downstream framing decision for Cap034 proxy wire (known-length keep-alive).
//!
//! Cap034 historically forced `Transfer-Encoding: chunked` + `Connection: close` on every
//! wire response (one-shot admission / LA-001 containment). That is not required by HTTP
//! for known-length pass-through. When upstream declares a single valid `Content-Length`
//! and the body is not transformed, emit CL + keep-alive so clients can reuse the socket.
//! Unknown length / streaming keeps Cap034 chunked+close.
//! Client `Connection: close` forces close even when length is known (CL + close, not TE).

use http::header::{CONTENT_LENGTH, TRANSFER_ENCODING};
use http::response::Parts;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownstreamFraming {
    /// Cap034 default: TE chunked + Connection: close (one-shot / unknown length).
    ChunkedClose,
    /// Known-length pass-through: Content-Length + Connection: keep-alive.
    KnownLengthKeepAlive { content_length: u64 },
    /// Known-length with client-requested close: Content-Length + Connection: close.
    KnownLengthClose { content_length: u64 },
}

/// Decide downstream framing from upstream response parts and client close intent.
///
/// Fail closed to [`DownstreamFraming::ChunkedClose`] when length is missing, ambiguous,
/// or `Transfer-Encoding` is still present.
pub fn decide_downstream_framing(parts: &Parts, client_close: bool) -> DownstreamFraming {
    if parts.headers.contains_key(TRANSFER_ENCODING) {
        return DownstreamFraming::ChunkedClose;
    }
    let mut values = parts.headers.get_all(CONTENT_LENGTH).iter();
    let Some(first) = values.next() else {
        return DownstreamFraming::ChunkedClose;
    };
    if values.next().is_some() {
        return DownstreamFraming::ChunkedClose;
    }
    let Ok(raw) = first.to_str() else {
        return DownstreamFraming::ChunkedClose;
    };
    let Ok(content_length) = raw.parse::<u64>() else {
        return DownstreamFraming::ChunkedClose;
    };
    if client_close {
        DownstreamFraming::KnownLengthClose { content_length }
    } else {
        DownstreamFraming::KnownLengthKeepAlive { content_length }
    }
}

impl DownstreamFraming {
    /// Content-Length when known-length framing was selected.
    pub fn content_length(self) -> Option<u64> {
        match self {
            DownstreamFraming::KnownLengthKeepAlive { content_length }
            | DownstreamFraming::KnownLengthClose { content_length } => Some(content_length),
            DownstreamFraming::ChunkedClose => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parts_with(headers: &[(&str, &str)]) -> Parts {
        let mut b = http::Response::builder().status(200);
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(()).expect("response").into_parts().0
    }

    #[test]
    fn known_length_selects_keepalive() {
        let parts = parts_with(&[("Content-Length", "1024")]);
        assert_eq!(
            decide_downstream_framing(&parts, false),
            DownstreamFraming::KnownLengthKeepAlive {
                content_length: 1024
            }
        );
    }

    #[test]
    fn known_length_client_close_selects_cl_close() {
        let parts = parts_with(&[("Content-Length", "1024")]);
        assert_eq!(
            decide_downstream_framing(&parts, true),
            DownstreamFraming::KnownLengthClose {
                content_length: 1024
            }
        );
    }

    #[test]
    fn missing_or_ambiguous_stays_chunked_close() {
        assert_eq!(
            decide_downstream_framing(&parts_with(&[]), false),
            DownstreamFraming::ChunkedClose
        );
        let mut parts = parts_with(&[("Content-Length", "10")]);
        parts
            .headers
            .append(CONTENT_LENGTH, http::HeaderValue::from_static("11"));
        assert_eq!(
            decide_downstream_framing(&parts, false),
            DownstreamFraming::ChunkedClose
        );
        assert_eq!(
            decide_downstream_framing(&parts_with(&[("Transfer-Encoding", "chunked")]), false),
            DownstreamFraming::ChunkedClose
        );
    }
}
