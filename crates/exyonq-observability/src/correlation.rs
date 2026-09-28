//! Request / trace correlation identities.

use getrandom::fill as getrandom_fill;

/// Product correlation identity for one admitted request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestIdentity {
    /// Validated client `x-request-id` when present and acceptable; never a security id.
    pub external_request_id: Option<String>,
    /// Always generated server-side; used for internal correlation / response echo by default.
    pub internal_request_id: String,
}

/// Generate a opaque internal request id (16 random bytes hex).
pub fn generate_internal_request_id() -> String {
    let mut buf = [0u8; 16];
    let _ = getrandom_fill(&mut buf);
    let mut out = String::with_capacity(32);
    for b in buf {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn valid_external_request_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 128 {
        return None;
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':' | b'/'))
    {
        return None;
    }
    Some(s.to_string())
}

/// Resolve identities from an optional raw `x-request-id` header value.
pub fn resolve_request_identity(raw_x_request_id: Option<&str>) -> RequestIdentity {
    let external = raw_x_request_id.and_then(valid_external_request_id);
    RequestIdentity {
        external_request_id: external,
        internal_request_id: generate_internal_request_id(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_control_chars() {
        let id = resolve_request_identity(Some("bad\nid"));
        assert!(id.external_request_id.is_none());
        assert_eq!(id.internal_request_id.len(), 32);
    }

    #[test]
    fn accepts_safe_external() {
        let id = resolve_request_identity(Some("req-abc_01"));
        assert_eq!(id.external_request_id.as_deref(), Some("req-abc_01"));
    }
}
