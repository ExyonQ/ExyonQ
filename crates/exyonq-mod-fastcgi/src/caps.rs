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
//! Aggregate FastCGI caps — PR5-B1-real production limits.
//!
//! Fail-closed: exceeding any cap returns an error; no silent truncation.

use std::time::Duration;

/// Maximum aggregate PARAMS body bytes across all PARAMS records.
pub const MAX_FCGI_PARAMS_BYTES: usize = 128 * 1024;

/// Maximum aggregate STDIN body bytes across all STDIN records.
pub const MAX_FCGI_STDIN_BYTES: usize = 32 * 1024 * 1024;

/// Maximum aggregate STDOUT bytes read from the peer.
pub const MAX_FCGI_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Maximum aggregate STDERR bytes read from the peer.
pub const MAX_FCGI_STDERR_BYTES: usize = 64 * 1024;

/// Maximum FastCGI records accepted per response.
pub const MAX_FCGI_RECORDS: usize = 10_000;

/// Maximum aggregate CGI header block bytes (PR5-B1).
pub const MAX_CGI_HEADER_BYTES: usize = 8192;

/// Maximum CGI response header count (PR5-B1).
pub const MAX_CGI_HEADER_COUNT: usize = 64;

/// Connect timeout for wire transport (distinct from read/write/delegate timeouts).
pub const FCGI_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// Read timeout per read operation on the wire.
pub const FCGI_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Write timeout per write operation on the wire.
pub const FCGI_WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// Cap enforcement failures (module boundary).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapError {
    ParamsExceeded { limit: usize, got: usize },
    StdinExceeded { limit: usize, got: usize },
    ResponseExceeded { limit: usize, got: usize },
    StderrExceeded { limit: usize, got: usize },
    RecordsExceeded { limit: usize, got: usize },
}

/// Reject if `current + delta` would exceed `limit`.
pub fn check_add_cap(limit: usize, current: usize, delta: usize) -> Result<usize, CapError> {
    let got = current.saturating_add(delta);
    if got > limit {
        return Err(cap_error_for_limit(limit, got));
    }
    Ok(got)
}

fn cap_error_for_limit(limit: usize, got: usize) -> CapError {
    if limit == MAX_FCGI_PARAMS_BYTES {
        CapError::ParamsExceeded { limit, got }
    } else if limit == MAX_FCGI_STDIN_BYTES {
        CapError::StdinExceeded { limit, got }
    } else if limit == MAX_FCGI_STDERR_BYTES {
        CapError::StderrExceeded { limit, got }
    } else if limit == MAX_FCGI_RECORDS {
        CapError::RecordsExceeded { limit, got }
    } else {
        CapError::ResponseExceeded { limit, got }
    }
}

/// Reject if `size` alone exceeds `limit`.
pub fn check_size_cap(limit: usize, size: usize) -> Result<(), CapError> {
    if size > limit {
        return Err(cap_error_for_limit(limit, size));
    }
    Ok(())
}
