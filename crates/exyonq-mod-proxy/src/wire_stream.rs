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
//! Adapters between module-api [`WireStream`] and Tokio async I/O.

use exyonq_module_api::proxy_wire::{BoxedWireStream, WireStream};
use std::io::{self, IoSlice};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

#[cfg(unix)]
use std::os::fd::{AsRawFd, RawFd};

struct TokioWireStream<S> {
    inner: S,
    #[cfg(unix)]
    peer_fd: Option<RawFd>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> WireStream for TokioWireStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let inner = self.get_mut();
        let mut read_buf = ReadBuf::new(buf);
        match Pin::new(&mut inner.inner).poll_read(cx, &mut read_buf) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(read_buf.filled().len())),
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write_vectored(cx, bufs)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }

    #[cfg(unix)]
    fn peer_tcp_fd(&self) -> Option<RawFd> {
        self.peer_fd
    }
}

/// Box a Tokio stream for the transport-agnostic module-api contract.
///
/// On Unix, when `S: AsRawFd`, prefer [`box_wire_stream_with_fd`] so platform
/// cork hooks can run. This entry point does not require `AsRawFd` (tests /
/// duplex mocks) and therefore cannot cork.
pub fn box_wire_stream<S>(stream: S) -> BoxedWireStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    Box::new(TokioWireStream {
        inner: stream,
        #[cfg(unix)]
        peer_fd: None,
    })
}

/// Box a stream while capturing the peer TCP fd for platform send hooks (cork).
#[cfg(unix)]
pub fn box_wire_stream_with_fd<S>(stream: S) -> BoxedWireStream
where
    S: AsyncRead + AsyncWrite + Unpin + Send + AsRawFd + 'static,
{
    let peer_fd = Some(stream.as_raw_fd());
    Box::new(TokioWireStream {
        inner: stream,
        peer_fd,
    })
}

struct BoxedWireIo(BoxedWireStream);

impl AsyncRead for BoxedWireIo {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let unfilled = buf.initialize_unfilled();
        match Pin::new(&mut *this.0).poll_read(cx, unfilled) {
            Poll::Ready(Ok(0)) => Poll::Ready(Ok(())),
            Poll::Ready(Ok(n)) => {
                // SAFETY: `poll_read` wrote `n` bytes into `unfilled`.
                unsafe {
                    buf.assume_init(n);
                }
                buf.advance(n);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for BoxedWireIo {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().0).poll_write(cx, data)
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().0).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        true
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().0).poll_shutdown(cx)
    }
}

/// Recover Tokio async I/O from a boxed wire stream handle.
pub(crate) fn into_async_io(stream: BoxedWireStream) -> impl AsyncRead + AsyncWrite + Unpin + Send {
    BoxedWireIo(stream)
}

/// Peer TCP fd from a boxed wire stream (Unix), if captured at box time.
#[cfg(unix)]
#[allow(dead_code)] // seam for cork diagnostics / future fd-aware product path
pub(crate) fn peer_tcp_fd_of(stream: &BoxedWireStream) -> Option<RawFd> {
    stream.peer_tcp_fd()
}
