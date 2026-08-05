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
//! P8T diagnostic StreamingWireWriter surface (mod-proxy local).
//! Feature `p8-transport-diag` only. Not a public product API.
//! No libc/socket2/nix. No SSE/OLS/bench knowledge.

use std::future::Future;
use std::io;
use std::pin::Pin;
use tokio::io::{AsyncWrite, AsyncWriteExt};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FlushPolicy {
    Immediate,
    Deferred,
    EndOfMessage,
    MoreComing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BackpressureState {
    Ready,
    WouldBlock,
    Closed,
}

/// Transport-only writer. Call site owns HTTP framing bytes.
pub(crate) trait StreamingWireWriter: Send + Unpin {
    fn write_headers<'a>(
        &'a mut self,
        bytes: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    fn write_body_chunk<'a>(
        &'a mut self,
        bytes: &'a [u8],
        flush_now: bool,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    fn write_body_end<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    fn flush_policy(&self) -> FlushPolicy;

    fn shutdown<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;

    fn backpressure_state(&self) -> BackpressureState;
}

/// Generic Tokio AsyncWrite fallback — default product-compatible path.
pub(crate) struct GenericAsyncWireWriter<'a, S> {
    stream: &'a mut S,
    policy: FlushPolicy,
    bp: BackpressureState,
}

impl<'a, S: AsyncWrite + Unpin + Send> GenericAsyncWireWriter<'a, S> {
    pub(crate) fn new(stream: &'a mut S) -> Self {
        Self {
            stream,
            policy: FlushPolicy::Immediate,
            bp: BackpressureState::Ready,
        }
    }
}

impl<'a, S: AsyncWrite + Unpin + Send> StreamingWireWriter for GenericAsyncWireWriter<'a, S> {
    fn write_headers<'b>(
        &'b mut self,
        bytes: &'b [u8],
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move {
            match self.stream.write_all(bytes).await {
                Ok(()) => {
                    self.bp = BackpressureState::Ready;
                    Ok(())
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    self.bp = BackpressureState::WouldBlock;
                    Err(err)
                }
                Err(err) => {
                    self.bp = BackpressureState::Closed;
                    Err(err)
                }
            }
        })
    }

    fn write_body_chunk<'b>(
        &'b mut self,
        bytes: &'b [u8],
        flush_now: bool,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move {
            self.stream.write_all(bytes).await?;
            if flush_now {
                self.stream.flush().await?;
            }
            self.bp = BackpressureState::Ready;
            Ok(())
        })
    }

    fn write_body_end<'b>(
        &'b mut self,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move {
            self.stream.write_all(b"0\r\n\r\n").await?;
            self.stream.flush().await?;
            self.policy = FlushPolicy::EndOfMessage;
            self.bp = BackpressureState::Ready;
            Ok(())
        })
    }

    fn flush_policy(&self) -> FlushPolicy {
        self.policy
    }

    fn shutdown<'b>(&'b mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'b>> {
        Box::pin(async move {
            self.stream.shutdown().await?;
            self.bp = BackpressureState::Closed;
            Ok(())
        })
    }

    fn backpressure_state(&self) -> BackpressureState {
        self.bp
    }
}

/// Finite-response diagnostic: one write_all for headers+TE frame+trailer.
/// Wire bytes identical to multi-write TE path. Not progressive-safe.
pub(crate) async fn write_finite_te_single_buffer<S: AsyncWrite + Unpin>(
    stream: &mut S,
    headers: &[u8],
    body: &[u8],
) -> io::Result<()> {
    let mut hex = [0u8; 24];
    let hex_len = crate::wire_conn::write_chunk_hex_len(body.len(), &mut hex);
    let total = headers.len() + hex_len + 2 + body.len() + 2 + 5;
    let mut wire = Vec::with_capacity(total);
    wire.extend_from_slice(headers);
    wire.extend_from_slice(&hex[..hex_len]);
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(body);
    wire.extend_from_slice(b"\r\n");
    wire.extend_from_slice(b"0\r\n\r\n");
    stream.write_all(&wire).await?;
    stream.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finite_single_buffer_matches_te_shape() {
        let mut out = Vec::new();
        write_finite_te_single_buffer(
            &mut out,
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"data: x\n\n",
        )
        .await
        .expect("write");
        let s = String::from_utf8(out).expect("utf8");
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("\r\ndata: x\n\n\r\n"));
        assert!(s.ends_with("0\r\n\r\n"));
    }

    #[tokio::test]
    async fn generic_writer_writes_headers() {
        let mut buf = Vec::new();
        {
            let mut w = GenericAsyncWireWriter::new(&mut buf);
            w.write_headers(b"HTTP/1.1 200 OK\r\n\r\n")
                .await
                .expect("headers");
            assert_eq!(w.flush_policy(), FlushPolicy::Immediate);
            assert_eq!(w.backpressure_state(), BackpressureState::Ready);
        }
        assert_eq!(&buf, b"HTTP/1.1 200 OK\r\n\r\n");
    }
}
