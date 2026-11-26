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
use crate::error::CtrlError;
use ::http::HeaderMap;
use std::path::Path;

pub const SERVER_TOKEN_HEADER: &str = "X-ExyonQ-Server-Token";
pub const SERVER_TOKEN_PREFIX: &str = "exq_sk_v1_";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Read,
    Write,
}

impl Scope {
    pub fn allows(self, method: &::http::Method) -> bool {
        match self {
            Self::Read => method == ::http::Method::GET || method == ::http::Method::HEAD,
            Self::Write => method != ::http::Method::GET && method != ::http::Method::HEAD,
        }
    }
}

pub fn validate_configured_token(token: impl AsRef<[u8]>) -> Result<Vec<u8>, CtrlError> {
    let bytes = token.as_ref();
    if bytes.starts_with(SERVER_TOKEN_PREFIX.as_bytes()) && bytes.len() > SERVER_TOKEN_PREFIX.len()
    {
        Ok(bytes.to_vec())
    } else {
        Err(CtrlError::Unauthorized)
    }
}

pub fn authorize(
    headers: &HeaderMap,
    scope: Scope,
    full: &[u8],
    read_only: Option<&[u8]>,
    write_only: Option<&[u8]>,
) -> Result<(), CtrlError> {
    let Some(raw) = headers.get(SERVER_TOKEN_HEADER) else {
        return Err(CtrlError::Unauthorized);
    };
    let Ok(presented) = raw.to_str() else {
        return Err(CtrlError::Unauthorized);
    };
    let presented = presented.as_bytes();
    let mut known = false;
    if constant_time_token_eq(presented, full) {
        return Ok(());
    }
    known |= read_only.is_some_and(|token| constant_time_token_eq(presented, token));
    known |= write_only.is_some_and(|token| constant_time_token_eq(presented, token));
    if !known {
        return Err(CtrlError::Unauthorized);
    }
    if read_only.is_some_and(|token| constant_time_token_eq(presented, token))
        && scope == Scope::Read
    {
        return Ok(());
    }
    if write_only.is_some_and(|token| constant_time_token_eq(presented, token))
        && scope == Scope::Write
    {
        return Ok(());
    }
    Err(CtrlError::Forbidden)
}

/// F-10: a rotating credential file must be owner-only (0600) and contain one
/// token. Reloaded per request so an atomic replacement rotates without a
/// restart; any unsafe mode fails closed.
pub fn read_token_file(path: &Path) -> Result<Vec<u8>, CtrlError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)
            .map_err(|_| CtrlError::Unauthorized)?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            return Err(CtrlError::Unauthorized);
        }
    }
    let raw = std::fs::read_to_string(path).map_err(|_| CtrlError::Unauthorized)?;
    validate_configured_token(raw.trim().as_bytes())
}

pub fn constant_time_token_eq(presented: &[u8], expected: &[u8]) -> bool {
    let max_len = presented.len().max(expected.len());
    let mut diff = presented.len() ^ expected.len();
    for idx in 0..max_len {
        let left = *presented.get(idx).unwrap_or(&0);
        let right = *expected.get(idx).unwrap_or(&0);
        diff |= usize::from(left ^ right);
    }
    let family_ok = presented.starts_with(SERVER_TOKEN_PREFIX.as_bytes())
        & expected.starts_with(SERVER_TOKEN_PREFIX.as_bytes());
    diff == 0 && family_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::http::HeaderValue;

    #[test]
    fn auth_is_fail_closed_without_header() {
        let headers = HeaderMap::new();
        assert!(matches!(
            authorize(&headers, Scope::Read, b"exq_sk_v1_expected", None, None),
            Err(CtrlError::Unauthorized)
        ));
    }

    #[test]
    fn auth_rejects_wrong_family_even_when_bytes_match() {
        assert!(!constant_time_token_eq(b"other", b"other"));
        assert!(validate_configured_token("not_sk").is_err());
    }

    #[test]
    fn auth_accepts_matching_server_token() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TOKEN_HEADER,
            HeaderValue::from_static("exq_sk_v1_expected"),
        );
        assert!(authorize(&headers, Scope::Read, b"exq_sk_v1_expected", None, None).is_ok());
    }

    #[test]
    fn f10_scopes_separate_read_and_write_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(
            SERVER_TOKEN_HEADER,
            HeaderValue::from_static("exq_sk_v1_read"),
        );
        assert!(authorize(
            &headers,
            Scope::Read,
            b"exq_sk_v1_full",
            Some(b"exq_sk_v1_read"),
            Some(b"exq_sk_v1_write"),
        )
        .is_ok());
        assert!(matches!(
            authorize(
                &headers,
                Scope::Write,
                b"exq_sk_v1_full",
                Some(b"exq_sk_v1_read"),
                Some(b"exq_sk_v1_write"),
            ),
            Err(CtrlError::Forbidden)
        ));
    }
}
