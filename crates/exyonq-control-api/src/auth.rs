use crate::error::CtrlError;
use ::http::HeaderMap;

pub const SERVER_TOKEN_HEADER: &str = "X-ExyonQ-Server-Token";
pub const SERVER_TOKEN_PREFIX: &str = "exq_sk_v1_";

pub fn validate_configured_token(token: impl AsRef<[u8]>) -> Result<Vec<u8>, CtrlError> {
    let bytes = token.as_ref();
    if bytes.starts_with(SERVER_TOKEN_PREFIX.as_bytes()) && bytes.len() > SERVER_TOKEN_PREFIX.len()
    {
        Ok(bytes.to_vec())
    } else {
        Err(CtrlError::Unauthorized)
    }
}

pub fn authorize(headers: &HeaderMap, expected: &[u8]) -> Result<(), CtrlError> {
    let Some(raw) = headers.get(SERVER_TOKEN_HEADER) else {
        return Err(CtrlError::Unauthorized);
    };
    let Ok(presented) = raw.to_str() else {
        return Err(CtrlError::Unauthorized);
    };
    if constant_time_token_eq(presented.as_bytes(), expected) {
        Ok(())
    } else {
        Err(CtrlError::Unauthorized)
    }
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
            authorize(&headers, b"exq_sk_v1_expected"),
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
        assert!(authorize(&headers, b"exq_sk_v1_expected").is_ok());
    }
}
