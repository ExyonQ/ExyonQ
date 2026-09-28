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
//! P8T diagnostic finite TE single-buffer writer (mod-proxy local).
//! Feature `p8-transport-diag` only. Not a public product API.
//! No libc/socket2/nix. No SSE/OLS/bench knowledge.
//!
//! Prior `StreamingWireWriter` / `GenericAsyncWireWriter` trait surface was
//! characterization-only and unused on the production diag path; removed to
//! keep `--all-features` clippy clean without `allow(dead_code)`.

use std::io;
use tokio::io::{AsyncWrite, AsyncWriteExt};

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
}
