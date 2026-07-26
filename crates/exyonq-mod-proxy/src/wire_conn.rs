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
//! Raw HTTP/1.1 loop for cached `/api/*` proxy paths (P4/P8 bench hot path) — KD3.4 module-owned.

use crate::hyper_forward::response_is_event_stream;
use crate::runtime::ProxyRuntime;
use crate::wire_io;
use bytes::Bytes;
use http_body_util::BodyExt;
use hyper::header::HeaderValue;
use std::io::{self, Result};
use std::sync::{Arc, OnceLock, RwLock};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

type BoxBody = http_body_util::combinators::BoxBody<Bytes, hyper::Error>;

/// Larger batches reduce flush/syscall overhead on long SSE streams (P8).
const SSE_FLUSH_CHUNK: usize = 16384;
#[cfg(feature = "p8t-fix-baseline")]
const SSE_FLUSH_INTERVAL: usize = 2;

static RUNTIME: OnceLock<RwLock<Arc<ProxyRuntime>>> = OnceLock::new();

pub fn pin_runtime(runtime: Arc<ProxyRuntime>) {
    match RUNTIME.get() {
        Some(lock) => {
            *lock.write().expect("proxy runtime pin poisoned") = runtime;
        }
        None => {
            let _ = RUNTIME.set(RwLock::new(runtime));
        }
    }
}

fn runtime() -> Arc<ProxyRuntime> {
    RUNTIME
        .get()
        .expect("ProxyRuntime not pinned — call install_kernel_hooks from CLI")
        .read()
        .expect("proxy runtime pin poisoned")
        .clone()
}

/// Pinned module runtime (composition root / cache load by cluster_id).
pub fn pinned_runtime() -> Arc<ProxyRuntime> {
    runtime()
}

fn header_field_count(head: &[u8], needle: &[u8]) -> usize {
    head.windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}

/// Raw-head WebSocket upgrade detection — wire proxy cannot tunnel upgrades.
fn wire_head_is_websocket_upgrade(head: &[u8]) -> bool {
    let h = head.to_ascii_lowercase();
    h.windows(b"upgrade: websocket".len())
        .any(|w| w == b"upgrade: websocket")
        && (h
            .windows(b"connection: upgrade".len())
            .any(|w| w == b"connection: upgrade")
            || h.windows(b"connection: keep-alive, upgrade".len())
                .any(|w| w == b"connection: keep-alive, upgrade"))
}

pub fn might_use_proxy_wire(head: &[u8]) -> bool {
    if !(head.starts_with(b"GET /api/") || head.starts_with(b"GET /api ")) {
        return false;
    }
    if wire_head_is_websocket_upgrade(head) {
        return false;
    }
    if header_field_count(head, b"Content-Length:") > 1 {
        return false;
    }
    if head
        .windows(b"Transfer-Encoding:".len())
        .any(|w| w == b"Transfer-Encoding:")
        && head
            .windows(b"Content-Length:".len())
            .any(|w| w == b"Content-Length:")
    {
        return false;
    }
    true
}

fn parse_get_path_and_query(head: &[u8]) -> Option<&str> {
    if !head.starts_with(b"GET ") {
        return None;
    }
    let rest = &head[4..];
    let end = rest.iter().position(|&b| b == b' ')?;
    std::str::from_utf8(&rest[..end]).ok()
}

pub async fn write_proxy_response_wire<S: AsyncWrite + Unpin>(
    stream: &mut S,
    response: hyper::Response<BoxBody>,
) -> Result<()> {
    let (parts, body) = response.into_parts();
    if response_is_event_stream(&parts) {
        return write_streaming_proxy_response_wire(stream, parts, body).await;
    }

    let body = body
        .collect()
        .await
        .map_err(|_| io::Error::other("upstream body error"))?
        .to_bytes();

    let mut wire = Vec::with_capacity(192 + body.len());
    wire.extend_from_slice(format!("HTTP/1.1 {}\r\n", parts.status.as_str()).as_bytes());
    for (name, value) in parts.headers.iter() {
        if name == hyper::header::CONNECTION
            || name == hyper::header::TRANSFER_ENCODING
            || name == hyper::header::CONTENT_LENGTH
        {
            continue;
        }
        if let Ok(value) = value.to_str() {
            wire.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
    }
    wire.extend_from_slice(format!("Content-Length: {}\r\n", body.len()).as_bytes());
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(&body);
    wire_io::write_all_async(stream, &wire).await
}

async fn write_streaming_proxy_response_wire<S: AsyncWrite + Unpin>(
    stream: &mut S,
    parts: http::response::Parts,
    mut body: BoxBody,
) -> Result<()> {
    #[cfg(feature = "p8-app-attribution")]
    crate::p8_app_attribution::note_request();
    #[cfg(feature = "p8-app-attribution")]
    let handler_timer = crate::p8_app_attribution::ScopeTimer::start();

    let mut wire = Vec::with_capacity(256);
    wire.extend_from_slice(format!("HTTP/1.1 {}\r\n", parts.status.as_str()).as_bytes());
    for (name, value) in parts.headers.iter() {
        if name == hyper::header::CONNECTION
            || name == hyper::header::TRANSFER_ENCODING
            || name == hyper::header::CONTENT_LENGTH
        {
            continue;
        }
        if let Ok(value) = value.to_str() {
            wire.extend_from_slice(format!("{name}: {value}\r\n").as_bytes());
        }
    }
    wire.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");

    // Diagnostic-only full-buffer path (P8T STEP_3). Not product default.
    #[cfg(feature = "p8-transport-diag")]
    {
        let mut pending = Vec::new();
        while let Some(frame) = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?
        {
            if let Ok(chunk) = frame.into_data() {
                pending.extend_from_slice(&chunk);
            }
        }
        crate::streaming_wire_writer::write_finite_te_single_buffer(stream, &wire, &pending)
            .await?;
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
        return Ok(());
    }

    // P8T-FIX A/B baseline: pre-fix 16 KiB batching (holds frames until SSE_FLUSH_CHUNK).
    #[cfg(all(
        not(feature = "p8-transport-diag"),
        feature = "p8t-fix-baseline"
    ))]
    {
        wire_io::write_all_async(stream, &wire).await?;
        let mut pending = Vec::new();
        let mut chunks_since_flush = 0usize;
        loop {
            let frame = body
                .frame()
                .await
                .transpose()
                .map_err(|_| io::Error::other("upstream body frame error"))?;
            let Some(frame) = frame else {
                break;
            };
            if let Ok(chunk) = frame.into_data() {
                pending.extend_from_slice(&chunk);
                while pending.len() >= SSE_FLUSH_CHUNK {
                    write_chunk(stream, &pending[..SSE_FLUSH_CHUNK], false).await?;
                    pending.drain(..SSE_FLUSH_CHUNK);
                    chunks_since_flush += 1;
                    if chunks_since_flush >= SSE_FLUSH_INTERVAL {
                        stream.flush().await?;
                        chunks_since_flush = 0;
                    }
                }
            }
        }
        if !pending.is_empty() {
            write_chunk(stream, &pending, true).await?;
        }
        stream.write_all(b"0\r\n\r\n").await?;
        stream.flush().await?;
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
        return Ok(());
    }

    // P8T-FIX product default: progressive per-frame TE write + flush.
    #[cfg(all(
        not(feature = "p8-transport-diag"),
        not(feature = "p8t-fix-baseline")
    ))]
    {
        write_streaming_te_progressive(stream, &wire, &mut body).await?;
        #[cfg(feature = "p8-app-attribution")]
        {
            let total = handler_timer.elapsed_ns();
            crate::p8_app_attribution::note_completed_stream(total, total);
        }
    }
    Ok(())
}

/// P8T-FIX product path: progressive-safe stream writer.
///
/// Emits each upstream data frame immediately (TE frame+data via `write_chunk`)
/// so sparse SSE events are not held until `SSE_FLUSH_CHUNK`.
///
/// Finite Ready-drain coalesce (EXPERIMENT_4) was measured on Netcup P8O-A and
/// **regressed** vs 16 KiB-batch coalesce baseline (~−7% RPS). It is not the
/// product default — retained only as a unit-test helper. Competitive
/// single-write shape remains diagnostic (`p8-transport-diag`) or future
/// platform writev/cork (separate admit).
async fn write_streaming_te_progressive<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &mut BoxBody,
) -> Result<()> {
    wire_io::write_all_async(stream, headers).await?;

    loop {
        let frame = body
            .frame()
            .await
            .transpose()
            .map_err(|_| io::Error::other("upstream body frame error"))?;
        let Some(frame) = frame else {
            break;
        };
        if let Ok(chunk) = frame.into_data() {
            if chunk.is_empty() {
                continue;
            }
            let mut offset = 0usize;
            while offset < chunk.len() {
                let end = (offset + SSE_FLUSH_CHUNK).min(chunk.len());
                let last = end == chunk.len();
                write_chunk(stream, &chunk[offset..end], last).await?;
                offset = end;
            }
        }
    }

    stream.write_all(b"0\r\n\r\n").await?;
    stream.flush().await?;
    Ok(())
}

/// Retained for unit tests + future finite-cap experiments (not product default).
#[cfg(test)]
async fn write_finite_headers_frame_trailer<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &[u8],
) -> Result<()> {
    let mut hex = [0u8; 24];
    let hex_len = write_chunk_hex_len(body.len(), &mut hex);
    let total = headers.len() + hex_len + 2 + body.len() + 2 + 5;
    const STACK_MAX: usize = 8192;
    if total <= STACK_MAX {
        let mut wire = [0u8; STACK_MAX];
        let mut n = 0usize;
        wire[n..n + headers.len()].copy_from_slice(headers);
        n += headers.len();
        if !body.is_empty() {
            wire[n..n + hex_len].copy_from_slice(&hex[..hex_len]);
            n += hex_len;
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
            wire[n..n + body.len()].copy_from_slice(body);
            n += body.len();
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
        }
        wire[n..n + 5].copy_from_slice(b"0\r\n\r\n");
        n += 5;
        stream.write_all(&wire[..n]).await?;
    } else {
        let mut wire = Vec::with_capacity(total);
        wire.extend_from_slice(headers);
        if !body.is_empty() {
            wire.extend_from_slice(&hex[..hex_len]);
            wire.extend_from_slice(b"\r\n");
            wire.extend_from_slice(body);
            wire.extend_from_slice(b"\r\n");
        }
        wire.extend_from_slice(b"0\r\n\r\n");
        stream.write_all(&wire).await?;
    }
    stream.flush().await?;
    Ok(())
}
pub(crate) fn write_chunk_hex_len(len: usize, out: &mut [u8; 24]) -> usize {
    if len == 0 {
        out[0] = b'0';
        return 1;
    }
    let mut digits = [0u8; 16];
    let mut n = len;
    let mut count = 0usize;
    while n > 0 {
        let d = (n & 0xf) as u8;
        digits[count] = b"0123456789abcdef"[d as usize];
        n >>= 4;
        count += 1;
    }
    for i in 0..count {
        out[i] = digits[count - 1 - i];
    }
    count
}

async fn write_chunk<S: AsyncWrite + Unpin>(
    stream: &mut S,
    data: &[u8],
    flush: bool,
) -> Result<()> {
    // Default: one write_all per TE chunk (coalesced contiguous buffer).
    // Feature p8tp-vectored: writev hex|CRLF|data|CRLF without assemble (EXPERIMENT_2).
    #[cfg(feature = "p8-app-attribution")]
    let frame_timer = crate::p8_app_attribution::ScopeTimer::start();
    let mut header = [0u8; 24];
    let hex_len = write_chunk_hex_len(data.len(), &mut header);
    #[cfg(feature = "p8-app-attribution")]
    let frame_ns = frame_timer.elapsed_ns();
    #[cfg(feature = "p8-app-attribution")]
    let write_timer = crate::p8_app_attribution::ScopeTimer::start();

    #[cfg(feature = "p8tp-vectored")]
    let write_result = {
        use std::io::IoSlice;
        let mut bufs = [
            IoSlice::new(&header[..hex_len]),
            IoSlice::new(b"\r\n"),
            IoSlice::new(data),
            IoSlice::new(b"\r\n"),
        ];
        // Tokio AsyncWrite has write_vectored; advance slices until drained.
        let mut slices: &mut [IoSlice<'_>] = &mut bufs;
        let mut result = Ok(());
        while !slices.is_empty() {
            match stream.write_vectored(slices).await {
                Ok(0) => {
                    result = Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write_vectored returned 0",
                    ));
                    break;
                }
                Ok(n) => IoSlice::advance_slices(&mut slices, n),
                Err(err) => {
                    result = Err(err);
                    break;
                }
            }
        }
        result
    };

    #[cfg(not(feature = "p8tp-vectored"))]
    let write_result = {
        let total = hex_len + 2 + data.len() + 2;
        const STACK_MAX: usize = 512;
        if total <= STACK_MAX {
            let mut wire = [0u8; STACK_MAX];
            let mut n = 0usize;
            wire[n..n + hex_len].copy_from_slice(&header[..hex_len]);
            n += hex_len;
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
            wire[n..n + data.len()].copy_from_slice(data);
            n += data.len();
            wire[n..n + 2].copy_from_slice(b"\r\n");
            n += 2;
            stream.write_all(&wire[..n]).await
        } else {
            let mut wire = Vec::with_capacity(total);
            wire.extend_from_slice(&header[..hex_len]);
            wire.extend_from_slice(b"\r\n");
            wire.extend_from_slice(data);
            wire.extend_from_slice(b"\r\n");
            stream.write_all(&wire).await
        }
    };

    #[cfg(feature = "p8-app-attribution")]
    {
        let write_ns = write_timer.elapsed_ns();
        if write_result.is_err() {
            crate::p8_app_attribution::note_client_disconnect();
        } else {
            crate::p8_app_attribution::note_write_chunk(data.len(), frame_ns, write_ns, flush);
        }
    }
    write_result?;
    if flush {
        #[cfg(feature = "p8-app-attribution")]
        let flush_timer = crate::p8_app_attribution::ScopeTimer::start();
        let flush_result = stream.flush().await;
        #[cfg(feature = "p8-app-attribution")]
        crate::p8_app_attribution::note_flush(flush_timer.elapsed_ns());
        flush_result?;
    }
    Ok(())
}

pub async fn serve<S>(
    cluster_id: u32,
    generation: u64,
    mut stream: S,
    first_head: Bytes,
    mut carry: Bytes,
    x_forwarded_for: HeaderValue,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let rt = runtime();
    let mut pending_head = Some(first_head);
    loop {
        if rt.cluster_generation() != generation {
            break;
        }
        let head = if let Some(head) = pending_head.take() {
            head
        } else {
            carry = ensure_headers(&mut stream, carry).await?;
            if !headers_complete(&carry) {
                break;
            }
            let (head, rest) = split_head(carry)?;
            carry = rest;
            head
        };

        let Some(path_and_query) = parse_get_path_and_query(head.as_ref()) else {
            break;
        };

        let Some(target) = rt.upstream_for_cluster(cluster_id) else {
            wire_io::write_all_async(
                &mut stream,
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nnot found",
            )
            .await?;
            break;
        };

        let response = rt
            .forward_get_for_wire(&target, path_and_query, Some(&x_forwarded_for))
            .await;
        write_proxy_response_wire(&mut stream, response).await?;
    }
    Ok(())
}

fn headers_complete(data: &[u8]) -> bool {
    wire_io::find_header_end(data).is_some()
}

async fn ensure_headers<S: AsyncRead + Unpin>(stream: &mut S, carry: Bytes) -> Result<Bytes> {
    match tokio::time::timeout(
        wire_io::header_read_timeout(),
        ensure_headers_inner(stream, carry),
    )
    .await
    {
        Ok(result) => result,
        Err(_) => Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "header read timeout",
        )),
    }
}

async fn ensure_headers_inner<S: AsyncRead + Unpin>(
    stream: &mut S,
    mut carry: Bytes,
) -> Result<Bytes> {
    use tokio::io::AsyncReadExt;
    let mut buf = [0u8; 512];
    loop {
        if headers_complete(&carry) {
            return Ok(carry);
        }
        if carry.len() > wire_io::MAX_HEADER {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request headers too large",
            ));
        }
        let n = stream.read(&mut buf).await?;
        if n == 0 {
            return Ok(carry);
        }
        carry = wire_io::append_bytes(carry, &buf[..n]);
    }
}

fn split_head(data: Bytes) -> Result<(Bytes, Bytes)> {
    let Some(end) = wire_io::find_header_end(data.as_ref()) else {
        return Ok((data, Bytes::new()));
    };
    let head_len = end + 4;
    Ok((data.slice(0..head_len), data.slice(head_len..)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::ProxyRuntime;
    use exyonq_module_api::proxy_dispatch::ProxyCompiledSlot;
    use hyper::body::{Body, Frame};
    use std::pin::Pin;
    use std::task::{Context, Poll};
    use std::time::Duration;

    fn install_test_runtime() -> Arc<ProxyRuntime> {
        let rt = Arc::new(ProxyRuntime::new());
        pin_runtime(Arc::clone(&rt));
        rt.bind_compiled_slots(
            1,
            &[ProxyCompiledSlot {
                cluster_id: 0,
                upstream_name: "backend".into(),
                target: "http://127.0.0.1:9000".into(),
                timeout: Duration::from_millis(500),
            }],
        );
        rt
    }

    #[test]
    fn wire_proxy_get_api_resolves_predicate() {
        let head = b"GET /api/users HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert!(might_use_proxy_wire(head));
        let path = parse_get_path_and_query(head).expect("path");
        assert_eq!(path, "/api/users");
    }

    #[test]
    fn might_use_rejects_invalid_te_cl_combo() {
        let head =
            b"GET /api/x HTTP/1.1\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[test]
    fn might_use_rejects_websocket_upgrade() {
        let head = b"GET /api/ws-echo HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
        assert!(!might_use_proxy_wire(head));
    }

    #[tokio::test]
    async fn upstream_for_cluster_after_bind() {
        let rt = install_test_runtime();
        let target = rt.upstream_for_cluster(0).expect("cluster 0");
        assert!(target
            .uri_for("/api/health")
            .to_string()
            .contains("127.0.0.1"));
    }

    #[test]
    fn cluster_generation_tracks_bind() {
        let rt = install_test_runtime();
        assert_eq!(rt.cluster_generation(), 1);
        rt.bind_compiled_slots(
            2,
            &[ProxyCompiledSlot {
                cluster_id: 0,
                upstream_name: "backend".into(),
                target: "http://127.0.0.1:9001".into(),
                timeout: Duration::from_millis(500),
            }],
        );
        assert_eq!(rt.cluster_generation(), 2);
    }

    #[test]
    fn write_chunk_hex_len_encodes_sizes() {
        let mut buf = [0u8; 24];
        assert_eq!(write_chunk_hex_len(0, &mut buf), 1);
        assert_eq!(buf[0], b'0');
        assert_eq!(write_chunk_hex_len(12, &mut buf), 1);
        assert_eq!(buf[0], b'c');
        assert_eq!(write_chunk_hex_len(425, &mut buf), 3);
        assert_eq!(&buf[..3], b"1a9");
    }

    #[tokio::test]
    async fn write_chunk_emits_single_te_frame() {
        let mut out = Vec::new();
        write_chunk(&mut out, b"data: hello\n\n", false)
            .await
            .expect("write");
        assert_eq!(out, b"d\r\ndata: hello\n\n\r\n");
    }

    #[tokio::test]
    async fn write_finite_headers_frame_trailer_single_write() {
        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let body = b"data: chunk-0\n\ndata: chunk-1\n\n";
        let mut out = Vec::new();
        write_finite_headers_frame_trailer(&mut out, headers, body)
            .await
            .expect("write");
        let expected_prefix = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        assert!(out.starts_with(expected_prefix));
        assert!(out.ends_with(b"0\r\n\r\n"));
        assert!(out.windows(body.len()).any(|w| w == body));
    }

    #[tokio::test]
    async fn finite_helper_coalesces_under_cap() {
        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let payload = b"data: chunk-0\n\ndata: chunk-1\n\n";
        let mut out = Vec::new();
        write_finite_headers_frame_trailer(&mut out, headers, payload)
            .await
            .expect("finite");
        assert!(out.starts_with(headers));
        assert!(out.ends_with(b"0\r\n\r\n"));
        assert!(out.windows(payload.len()).any(|w| w == payload));
    }

    #[tokio::test]
    async fn progressive_emits_before_eos() {
        use std::convert::Infallible;
        use std::sync::atomic::{AtomicBool, Ordering};
        use tokio::sync::mpsc;

        struct ChanBody {
            rx: mpsc::Receiver<Option<Bytes>>,
        }
        impl Body for ChanBody {
            type Data = Bytes;
            type Error = Infallible;
            fn poll_frame(
                mut self: Pin<&mut Self>,
                cx: &mut Context<'_>,
            ) -> Poll<Option<std::result::Result<Frame<Self::Data>, Self::Error>>> {
                match self.rx.poll_recv(cx) {
                    Poll::Ready(Some(Some(b))) => Poll::Ready(Some(Ok(Frame::data(b)))),
                    Poll::Ready(Some(None)) | Poll::Ready(None) => Poll::Ready(None),
                    Poll::Pending => Poll::Pending,
                }
            }
        }

        struct ProbeWrite {
            inner: Vec<u8>,
            saw_one_before_two: Arc<AtomicBool>,
        }
        impl AsyncWrite for ProbeWrite {
            fn poll_write(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &[u8],
            ) -> Poll<std::result::Result<usize, io::Error>> {
                self.inner.extend_from_slice(buf);
                let s = String::from_utf8_lossy(&self.inner);
                if s.contains("data: one") && !s.contains("data: two") {
                    self.saw_one_before_two.store(true, Ordering::SeqCst);
                }
                Poll::Ready(Ok(buf.len()))
            }
            fn poll_flush(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
            fn poll_shutdown(
                self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
            ) -> Poll<std::result::Result<(), io::Error>> {
                Poll::Ready(Ok(()))
            }
        }

        let flag = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Option<Bytes>>(4);
        tokio::spawn(async move {
            let _ = tx
                .send(Some(Bytes::from_static(b"data: one\n\n")))
                .await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx
                .send(Some(Bytes::from_static(b"data: two\n\n")))
                .await;
            let _ = tx.send(None).await;
        });

        let headers = b"HTTP/1.1 200\r\nTransfer-Encoding: chunked\r\n\r\n";
        let mut body: BoxBody = ChanBody { rx }.map_err(|n| match n {}).boxed();
        let mut out = ProbeWrite {
            inner: Vec::new(),
            saw_one_before_two: Arc::clone(&flag),
        };
        write_streaming_te_progressive(&mut out, headers, &mut body)
            .await
            .expect("dual");
        let s = String::from_utf8_lossy(&out.inner);
        assert!(s.contains("data: one"));
        assert!(s.contains("data: two"));
        assert!(s.ends_with("0\r\n\r\n"));
        assert!(
            flag.load(Ordering::SeqCst),
            "expected progressive write of first event before second"
        );
    }
}
