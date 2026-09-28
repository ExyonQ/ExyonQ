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
//! Wire connection I/O helpers (KD3.4) — module-local only.

use bytes::Bytes;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Process-lifetime cache for `EXYONQ_READ_TIMEOUT_MS` (0 = unset; else millis + 1).
static HEADER_READ_TIMEOUT_MS_CACHE: AtomicU64 = AtomicU64::new(0);

pub const MAX_HEADER: usize = 8192;

pub fn find_header_end(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    let limit = buf.len().saturating_sub(3);
    while i < limit {
        if buf[i] == b'\r' && buf[i + 1] == b'\n' && buf[i + 2] == b'\r' && buf[i + 3] == b'\n' {
            return Some(i);
        }
        i += 1;
    }
    None
}

pub fn append_bytes(left: Bytes, right: &[u8]) -> Bytes {
    if left.is_empty() {
        return Bytes::copy_from_slice(right);
    }
    if right.is_empty() {
        return left;
    }
    if left.len() + right.len() <= 512 {
        let mut merged = [0u8; 512];
        merged[..left.len()].copy_from_slice(&left);
        merged[left.len()..left.len() + right.len()].copy_from_slice(right);
        return Bytes::copy_from_slice(&merged[..left.len() + right.len()]);
    }
    let mut merged = bytes::BytesMut::with_capacity(left.len() + right.len());
    merged.extend_from_slice(&left);
    merged.extend_from_slice(right);
    merged.freeze()
}

/// Header-read timeout from `EXYONQ_READ_TIMEOUT_MS` (process-lifetime cache).
///
/// Unset / invalid → 30s. Mid-process env mutation is not observed —
/// set before process start (tests may reset).
pub fn header_read_timeout() -> Duration {
    let cached = HEADER_READ_TIMEOUT_MS_CACHE.load(Ordering::Relaxed);
    if cached != 0 {
        return Duration::from_millis(cached - 1);
    }
    let d = std::env::var("EXYONQ_READ_TIMEOUT_MS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(30));
    let encoded = d.as_millis() as u64 + 1;
    let _ = HEADER_READ_TIMEOUT_MS_CACHE.compare_exchange(
        0,
        encoded,
        Ordering::Relaxed,
        Ordering::Relaxed,
    );
    let final_ms = HEADER_READ_TIMEOUT_MS_CACHE.load(Ordering::Relaxed) - 1;
    Duration::from_millis(final_ms)
}

/// Test-only: clear process-lifetime cache (simulates a fresh process).
#[cfg(any(test, feature = "test-utils"))]
#[doc(hidden)]
pub fn reset_header_read_timeout_cache_for_tests() {
    HEADER_READ_TIMEOUT_MS_CACHE.store(0, Ordering::Relaxed);
}

pub async fn write_all_async<S: AsyncWrite + Unpin>(stream: &mut S, buf: &[u8]) -> io::Result<()> {
    stream.write_all(buf).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn find_header_end_partial_crlf_not_ready() {
        assert!(find_header_end(b"GET / HTTP/1.1\r\nHost: x\r\n").is_none());
    }

    #[test]
    fn append_bytes_small_stack_merge() {
        let merged = append_bytes(Bytes::from_static(b"ab"), b"cd");
        assert_eq!(&merged[..], b"abcd");
    }

    #[test]
    fn append_bytes_large_heap_merge() {
        let left = Bytes::from(vec![b'a'; 400]);
        let right = vec![b'b'; 200];
        let merged = append_bytes(left, &right);
        assert_eq!(merged.len(), 600);
    }
}
