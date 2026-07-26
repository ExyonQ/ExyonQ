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
//! PREMAT-SEC-001 — static materialization budget (early reject + bounded read).

use exyonq_mod_static::{
    read_bounded_from_reader, read_file_bytes_with_budget, serve_file_sync, StaticError,
};
use exyonq_module_api::static_dispatch::MATERIALIZATION_BUDGET_EXCEEDED_HEADER;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use tempfile::NamedTempFile;

/// Reader that counts bytes actually pulled from the underlying source.
struct CountingReader<R> {
    inner: R,
    bytes_read: AtomicU64,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.bytes_read.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

#[test]
fn budget_none_preserves_full_read() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&vec![7u8; 2048]).unwrap();
    f.flush().unwrap();
    let resp = serve_file_sync(f.path()).expect("read");
    assert_eq!(resp.status(), hyper::StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get("content-length")
            .unwrap()
            .to_str()
            .unwrap(),
        "2048"
    );
}

#[test]
fn early_reject_metadata_over_budget_reads_zero_body_bytes() {
    let mut f = NamedTempFile::new().unwrap();
    // Sparse-friendly: write a small prefix then set_len large when supported.
    f.write_all(b"head").unwrap();
    f.as_file().set_len(1_048_576).unwrap();
    f.flush().unwrap();

    let err = read_file_bytes_with_budget(f.path(), Some(1024)).unwrap_err();
    assert!(matches!(err, StaticError::BudgetExceeded));
}

#[test]
fn exact_limit_succeeds() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&vec![1u8; 64]).unwrap();
    f.flush().unwrap();
    let (bytes, _) = read_file_bytes_with_budget(f.path(), Some(64)).expect("ok");
    assert_eq!(bytes.len(), 64);
}

#[test]
fn limit_minus_one_succeeds() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&vec![1u8; 63]).unwrap();
    f.flush().unwrap();
    let (bytes, _) = read_file_bytes_with_budget(f.path(), Some(64)).expect("ok");
    assert_eq!(bytes.len(), 63);
}

#[test]
fn limit_plus_one_budget_exceeded() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&vec![1u8; 65]).unwrap();
    f.flush().unwrap();
    let err = read_file_bytes_with_budget(f.path(), Some(64)).unwrap_err();
    assert!(matches!(err, StaticError::BudgetExceeded));
}

#[test]
fn empty_file_ok() {
    let f = NamedTempFile::new().unwrap();
    let (bytes, _) = read_file_bytes_with_budget(f.path(), Some(64)).expect("ok");
    assert!(bytes.is_empty());
}

#[test]
fn bounded_reader_stops_without_full_drain() {
    let data = vec![9u8; 10_000];
    let mut counting = CountingReader {
        inner: std::io::Cursor::new(data),
        bytes_read: AtomicU64::new(0),
    };
    let err = read_bounded_from_reader(&mut counting, 100).unwrap_err();
    assert!(matches!(err, StaticError::BudgetExceeded));
    let pulled = counting.bytes_read.load(Ordering::Relaxed);
    assert!(pulled <= 100 + 8192, "pulled={pulled}");
    assert!(pulled < 10_000, "must not drain EOF");
}

#[test]
fn budget_exceeded_error_is_distinct_and_header_constant_stable() {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(&vec![1u8; 200]).unwrap();
    f.flush().unwrap();
    match read_file_bytes_with_budget(f.path(), Some(16)) {
        Err(StaticError::BudgetExceeded) => {}
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
    assert_eq!(
        MATERIALIZATION_BUDGET_EXCEEDED_HEADER,
        "x-exyonq-materialization-budget-exceeded"
    );
}
