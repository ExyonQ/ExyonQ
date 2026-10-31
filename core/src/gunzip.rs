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
//! NG-18 / ADR-064 Phase B — streaming `gunzip` response filter.
//!
//! Mirrors nginx `gunzip on`: when the client does not accept gzip and the
//! origin sent `Content-Encoding: gzip`, decode the body while streaming.
//! Bounded output per poll (`buffer_bytes`), no whole-body buffering.

use bytes::{Buf, Bytes, BytesMut};
use flate2::{Crc, Decompress, FlushDecompress, Status};
use http_body::{Body, Frame, SizeHint};
use http_body_util::BodyExt;
use std::pin::Pin;
use std::task::{Context, Poll};

pub(crate) type HttpBoxBody =
    http_body_util::combinators::BoxBody<Bytes, exyonq_module_api::BoxError>;

/// Body error that aborts the response stream (Hyper closes the connection),
/// matching nginx's behaviour for an invalid gzip stream.
#[derive(Debug)]
pub(crate) struct GunzipError(&'static str);

impl std::fmt::Display for GunzipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for GunzipError {}

/// Apply `gunzip` to a response when the route enables it and the client does
/// not accept gzip. `HEAD` and non-200 responses are never transformed.
pub(crate) fn maybe_gunzip(
    enabled: bool,
    buffer_bytes: usize,
    accept_encoding: Option<&str>,
    method: &hyper::Method,
    response: &mut hyper::Response<HttpBoxBody>,
) {
    if !enabled {
        return;
    }
    if response.status() != hyper::StatusCode::OK {
        return;
    }
    let gzip = response
        .headers()
        .get(hyper::header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value.split(',').any(|token| {
                let token = token.trim();
                token.eq_ignore_ascii_case("gzip") || token.eq_ignore_ascii_case("x-gzip")
            })
        });
    if !gzip || client_accepts_gzip(accept_encoding) {
        return;
    }

    let headers = response.headers_mut();
    headers.remove(hyper::header::CONTENT_ENCODING);
    headers.remove(hyper::header::CONTENT_LENGTH);
    // nginx weakens a strong ETag when it changes the representation.
    if let Some(etag) = headers.get(hyper::header::ETAG).cloned() {
        if let Ok(value) = etag.to_str() {
            if !value.starts_with("W/") {
                if let Ok(weak) = hyper::header::HeaderValue::from_str(&format!("W/{value}")) {
                    headers.insert(hyper::header::ETAG, weak);
                }
            }
        }
    }

    // nginx strips the encoding headers on HEAD too, but there is no body to
    // decode.
    if *method == hyper::Method::HEAD {
        return;
    }
    let body = std::mem::replace(response.body_mut(), empty_body());
    *response.body_mut() = GunzipBody::new(body, buffer_bytes).boxed();
}

fn empty_body() -> HttpBoxBody {
    http_body_util::Full::new(Bytes::new())
        .map_err(|never| match never {})
        .boxed()
}

/// RFC 7231 `Accept-Encoding`: only an explicit gzip/x-gzip with q>0 counts.
/// nginx treats `*` as not accepting gzip for `gunzip` (differential oracle).
fn client_accepts_gzip(header: Option<&str>) -> bool {
    let Some(header) = header else {
        return false;
    };
    for part in header.split(',') {
        let mut params = part.split(';');
        let coding = params.next().unwrap_or("").trim();
        if !coding.eq_ignore_ascii_case("gzip") && !coding.eq_ignore_ascii_case("x-gzip") {
            continue;
        }
        let mut q = 1.0f32;
        for param in params {
            let param = param.trim();
            if let Some(value) = param
                .strip_prefix("q=")
                .or_else(|| param.strip_prefix("Q="))
            {
                q = value.trim().parse().unwrap_or(0.0);
            }
        }
        if q > 0.0 {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Header,
    Deflate,
    Trailer,
    Done,
}

struct Decoder {
    state: State,
    input: BytesMut,
    deflate: Decompress,
    crc: Crc,
    isize: u32,
    max_out: usize,
}

impl Decoder {
    fn new(max_out: usize) -> Self {
        Self {
            state: State::Header,
            input: BytesMut::new(),
            deflate: Decompress::new(false),
            crc: Crc::new(),
            isize: 0,
            max_out,
        }
    }

    fn feed(&mut self, data: &[u8], out: &mut BytesMut) -> Result<(), ()> {
        self.input.extend_from_slice(data);
        self.pump(out)
    }

    fn pump(&mut self, out: &mut BytesMut) -> Result<(), ()> {
        loop {
            if out.len() >= self.max_out {
                return Ok(());
            }
            match self.state {
                State::Header => {
                    let Some(consumed) = parse_header(&self.input)? else {
                        // Malformed-but-unterminated header fields must not
                        // buffer the whole upstream body.
                        if self.input.len() > 64 * 1024 {
                            return Err(());
                        }
                        return Ok(());
                    };
                    self.input.advance(consumed);
                    self.state = State::Deflate;
                }
                State::Deflate => {
                    if !decode_deflate(
                        &mut self.input,
                        &mut self.deflate,
                        &mut self.crc,
                        &mut self.isize,
                        out,
                        self.max_out,
                    )? {
                        return Ok(());
                    }
                    self.state = State::Trailer;
                }
                State::Trailer => {
                    if self.input.len() < 8 {
                        return Ok(());
                    }
                    let expected_crc = u32::from_le_bytes([
                        self.input[0],
                        self.input[1],
                        self.input[2],
                        self.input[3],
                    ]);
                    let expected_isize = u32::from_le_bytes([
                        self.input[4],
                        self.input[5],
                        self.input[6],
                        self.input[7],
                    ]);
                    if expected_crc != self.crc.sum() || expected_isize != self.isize {
                        return Err(());
                    }
                    self.input.advance(8);
                    if self.input.is_empty() {
                        self.state = State::Done;
                    } else {
                        // Concatenated gzip members: reset for the next one.
                        self.deflate = Decompress::new(false);
                        self.crc = Crc::new();
                        self.isize = 0;
                        self.state = State::Header;
                    }
                }
                State::Done => return Ok(()),
            }
        }
    }
}

/// Return the gzip header length, `None` when more bytes are needed, `Err` for
/// a malformed header. Optional fields are bounded to 64 KiB.
fn parse_header(buf: &[u8]) -> Result<Option<usize>, ()> {
    if buf.len() < 10 {
        return Ok(None);
    }
    if buf[0] != 0x1f || buf[1] != 0x8b || buf[2] != 8 || buf[3] & 0xe0 != 0 {
        return Err(());
    }
    let flags = buf[3];
    let mut pos = 10usize;
    if flags & 0x04 != 0 {
        if buf.len() < pos + 2 {
            return Ok(None);
        }
        let xlen = u16::from_le_bytes([buf[pos], buf[pos + 1]]) as usize;
        pos += 2;
        if buf.len() < pos + xlen {
            return Ok(None);
        }
        pos += xlen;
    }
    for flag in [0x08u8, 0x10u8] {
        if flags & flag != 0 {
            let Some(end) = buf[pos.min(buf.len())..].iter().position(|&b| b == 0) else {
                return Ok(None);
            };
            pos += end + 1;
        }
    }
    if flags & 0x02 != 0 {
        if buf.len() < pos + 2 {
            return Ok(None);
        }
        pos += 2;
    }
    if pos > 64 * 1024 {
        return Err(());
    }
    Ok(Some(pos))
}

/// Decode available raw-deflate input. Returns `true` on stream end.
fn decode_deflate(
    input: &mut BytesMut,
    deflate: &mut Decompress,
    crc: &mut Crc,
    isize: &mut u32,
    out: &mut BytesMut,
    max_out: usize,
) -> Result<bool, ()> {
    let mut scratch = [0u8; 16 * 1024];
    loop {
        if input.is_empty() {
            return Ok(false);
        }
        let before_in = deflate.total_in();
        let before_out = deflate.total_out();
        let status = deflate
            .decompress(&input[..], &mut scratch, FlushDecompress::None)
            .map_err(|_| ())?;
        let consumed = (deflate.total_in() - before_in) as usize;
        let produced = (deflate.total_out() - before_out) as usize;
        input.advance(consumed);
        if produced > 0 {
            crc.update(&scratch[..produced]);
            *isize = isize.wrapping_add(produced as u32);
            out.extend_from_slice(&scratch[..produced]);
        }
        match status {
            Status::StreamEnd => return Ok(true),
            Status::Ok | Status::BufError => {
                if consumed == 0 && produced == 0 {
                    return if status == Status::BufError {
                        Err(())
                    } else {
                        Ok(false)
                    };
                }
                if out.len() >= max_out {
                    return Ok(false);
                }
            }
        }
    }
}

struct GunzipBody {
    inner: HttpBoxBody,
    decoder: Decoder,
    out: BytesMut,
    trailers: Option<hyper::HeaderMap>,
    done: bool,
    inner_done: bool,
    error: Option<exyonq_module_api::BoxError>,
}

impl GunzipBody {
    fn new(inner: HttpBoxBody, buffer_bytes: usize) -> Self {
        Self {
            inner,
            decoder: Decoder::new(buffer_bytes.max(1024)),
            out: BytesMut::new(),
            trailers: None,
            done: false,
            inner_done: false,
            error: None,
        }
    }
}

impl Body for GunzipBody {
    type Data = Bytes;
    type Error = exyonq_module_api::BoxError;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        loop {
            if let Some(err) = this.error.take() {
                // Terminal: never regenerate the same error on the next poll.
                this.done = true;
                this.trailers = None;
                return Poll::Ready(Some(Err(err)));
            }
            if !this.out.is_empty() {
                return Poll::Ready(Some(Ok(Frame::data(this.out.split().freeze()))));
            }
            if this.done {
                return Poll::Ready(
                    this.trailers
                        .take()
                        .map(|trailers| Ok(Frame::trailers(trailers))),
                );
            }
            if this.inner_done {
                let before = this.out.len();
                if this.decoder.pump(&mut this.out).is_err() {
                    this.error = Some(Box::new(GunzipError("invalid gzip stream")));
                    continue;
                }
                if !this.out.is_empty() {
                    continue;
                }
                if this.decoder.state == State::Done {
                    this.done = true;
                    continue;
                }
                if this.decoder.input.is_empty() && before == this.out.len() {
                    this.error = Some(Box::new(GunzipError("truncated gzip stream")));
                }
                continue;
            }
            match Pin::new(&mut this.inner).poll_frame(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => {
                    this.inner_done = true;
                }
                Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                    Ok(data) => {
                        if this.decoder.feed(&data, &mut this.out).is_err() {
                            this.error = Some(Box::new(GunzipError("invalid gzip stream")));
                        }
                    }
                    Err(frame) => {
                        if let Ok(trailers) = frame.into_trailers() {
                            this.trailers = Some(trailers);
                        }
                    }
                },
                Poll::Ready(Some(Err(err))) => return Poll::Ready(Some(Err(err))),
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done && self.out.is_empty()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::Full;

    fn gzip(data: &[u8]) -> Vec<u8> {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn collect(body: GunzipBody) -> Vec<u8> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            let mut out = Vec::new();
            let mut body = std::pin::pin!(body);
            while let Some(frame) = std::future::poll_fn(|cx| body.as_mut().poll_frame(cx)).await {
                if let Ok(frame) = frame {
                    if let Ok(data) = frame.into_data() {
                        out.extend_from_slice(&data);
                    }
                }
            }
            out
        })
    }

    fn boxed(data: Vec<u8>) -> HttpBoxBody {
        Full::new(Bytes::from(data))
            .map_err(|never| match never {})
            .boxed()
    }

    #[test]
    fn decodes_single_member_across_chunks() {
        let plain = b"ng18-gunzip-unit-body".repeat(1024);
        let wire = gzip(&plain);
        let body = GunzipBody::new(boxed(wire), 4096);
        assert_eq!(collect(body), plain);
    }

    #[test]
    fn decodes_concatenated_members() {
        let a = b"first-member".repeat(100);
        let b = b"second-member".repeat(100);
        let mut wire = gzip(&a);
        wire.extend_from_slice(&gzip(&b));
        let body = GunzipBody::new(boxed(wire), 4096);
        let mut expected = a.clone();
        expected.extend_from_slice(&b);
        assert_eq!(collect(body), expected);
    }

    #[test]
    fn accept_encoding_matrix() {
        assert!(!client_accepts_gzip(None));
        assert!(!client_accepts_gzip(Some("identity")));
        assert!(!client_accepts_gzip(Some("gzip;q=0")));
        assert!(!client_accepts_gzip(Some("*")));
        assert!(client_accepts_gzip(Some("gzip")));
        assert!(client_accepts_gzip(Some("br, gzip;q=0.5")));
        assert!(client_accepts_gzip(Some("GZIP")));
    }

    #[test]
    fn invalid_gzip_ends_without_hanging() {
        let body = GunzipBody::new(boxed(b"not-a-gzip-stream".to_vec()), 4096);
        assert!(collect(body).is_empty());
    }
}
