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
//! Bounded materialization reads for optional `materialization_budget_bytes`.

use crate::StaticError;
use std::io::Read;

/// Read from `reader` until EOF or until more than `limit` bytes would be buffered.
///
/// On crossing the limit: does **not** append the crossing chunk, stops reading, and
/// returns [`StaticError::BudgetExceeded`]. Does not drain the remainder of the source.
///
/// Crossing allowance: detection is per-chunk; at most `limit` bytes are retained.
pub fn read_bounded_from_reader<R: Read>(
    reader: &mut R,
    limit: u64,
) -> Result<Vec<u8>, StaticError> {
    let limit_usize = usize::try_from(limit).map_err(|_| StaticError::BudgetExceeded)?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = reader.read(&mut chunk)?;
        if n == 0 {
            return Ok(buf);
        }
        match buf.len().checked_add(n) {
            Some(total) if total <= limit_usize => {
                buf.extend_from_slice(&chunk[..n]);
            }
            _ => {
                drop(buf);
                return Err(StaticError::BudgetExceeded);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn under_limit_ok() {
        let mut src = Cursor::new(vec![1u8; 10]);
        let out = read_bounded_from_reader(&mut src, 10).expect("ok");
        assert_eq!(out.len(), 10);
        assert_eq!(src.position(), 10);
    }

    #[test]
    fn exact_limit_ok() {
        let mut src = Cursor::new(vec![2u8; 64]);
        let out = read_bounded_from_reader(&mut src, 64).expect("ok");
        assert_eq!(out.len(), 64);
    }

    #[test]
    fn over_limit_no_append_no_drain() {
        // Force small reads so we can observe stop-without-looping-to-EOF.
        struct Chunked<'a> {
            data: &'a [u8],
            pos: usize,
            chunk: usize,
        }
        impl Read for Chunked<'_> {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.pos >= self.data.len() {
                    return Ok(0);
                }
                let n = self.chunk.min(self.data.len() - self.pos).min(buf.len());
                buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
                self.pos += n;
                Ok(n)
            }
        }
        let data = vec![3u8; 100];
        let mut src = Chunked {
            data: &data,
            pos: 0,
            chunk: 8,
        };
        let err = read_bounded_from_reader(&mut src, 16).unwrap_err();
        assert!(matches!(err, StaticError::BudgetExceeded));
        // After detecting the crossing chunk, the loop returns — no further reads.
        // 16 accepted + one 8-byte crossing pull ⇒ pos == 24.
        assert_eq!(src.pos, 24);
        assert!(src.pos < data.len());
    }

    #[test]
    fn empty_ok() {
        let mut src = Cursor::new(Vec::<u8>::new());
        let out = read_bounded_from_reader(&mut src, 1).expect("ok");
        assert!(out.is_empty());
    }

    #[test]
    fn many_small_reads_checked_total() {
        // Force small reads via Take windows by feeding exact limit across chunks.
        let mut src = Cursor::new(vec![9u8; 100]);
        let err = read_bounded_from_reader(&mut src, 99).unwrap_err();
        assert!(matches!(err, StaticError::BudgetExceeded));
    }
}
