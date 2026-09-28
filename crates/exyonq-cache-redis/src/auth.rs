//! WC7C — HMAC-SHA256 event authenticity (adapter wire envelope).

use exyonq_module_api::{
    CoordinationError, CoordinationRejectReason, InvalidationEvent, InvalidationOperation,
};
use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Active (+ optional previous) signing keys. Secrets never logged.
#[derive(Clone)]
pub struct EventSigningKeys {
    pub active_key_id: String,
    pub active_key: Vec<u8>,
    pub previous_key_id: Option<String>,
    pub previous_key: Option<Vec<u8>>,
}

impl std::fmt::Debug for EventSigningKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventSigningKeys")
            .field("active_key_id", &self.active_key_id)
            .field("active_key_set", &(!self.active_key.is_empty()))
            .field("previous_key_id", &self.previous_key_id)
            .field(
                "previous_key_set",
                &self.previous_key.as_ref().map(|k| !k.is_empty()),
            )
            .finish()
    }
}

impl EventSigningKeys {
    pub fn from_env() -> Result<Self, CoordinationError> {
        let active_key_id =
            std::env::var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID").unwrap_or_else(|_| "k1".into());
        let active_key = read_secret_bytes("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY")?;
        if active_key.is_empty() || active_key_id.is_empty() {
            return Err(CoordinationError::new(
                CoordinationRejectReason::ProviderUnavailable,
            ));
        }
        let previous_key_id = std::env::var("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_ID")
            .ok()
            .filter(|s| !s.is_empty());
        let previous_material_present = std::env::var("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY").is_ok()
            || std::env::var("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_FILE").is_ok();
        let previous_key = if previous_key_id.is_some() || previous_material_present {
            let key = read_secret_bytes("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY")?;
            if key.is_empty() || previous_key_id.is_none() {
                return Err(CoordinationError::new(
                    CoordinationRejectReason::InternalError,
                ));
            }
            Some(key)
        } else {
            None
        };
        if previous_key_id.is_some() != previous_key.is_some() {
            return Err(CoordinationError::new(
                CoordinationRejectReason::InternalError,
            ));
        }
        Ok(Self {
            active_key_id,
            active_key,
            previous_key_id,
            previous_key,
        })
    }

    pub fn for_tests(active_id: &str, active: &[u8]) -> Self {
        Self {
            active_key_id: active_id.into(),
            active_key: active.to_vec(),
            previous_key_id: None,
            previous_key: None,
        }
    }

    pub fn with_previous(mut self, id: &str, key: &[u8]) -> Self {
        self.previous_key_id = Some(id.into());
        self.previous_key = Some(key.to_vec());
        self
    }

    fn key_for_id(&self, key_id: &str) -> Option<&[u8]> {
        if key_id == self.active_key_id {
            return Some(&self.active_key);
        }
        if self.previous_key_id.as_deref() == Some(key_id) {
            return self.previous_key.as_deref();
        }
        None
    }
}

fn read_secret_bytes(env: &str) -> Result<Vec<u8>, CoordinationError> {
    // Deterministic precedence: `{env}_FILE` wins when set (even if `{env}` is also set).
    if let Ok(path) = std::env::var(format!("{env}_FILE")) {
        let path = path.trim();
        if path.is_empty() {
            return Err(CoordinationError::new(
                CoordinationRejectReason::ProviderUnavailable,
            ));
        }
        return std::fs::read(path)
            .map_err(|_| CoordinationError::new(CoordinationRejectReason::ProviderUnavailable));
    }
    std::env::var(env)
        .map(|s| s.into_bytes())
        .map_err(|_| CoordinationError::new(CoordinationRejectReason::ProviderUnavailable))
}

/// Replay / clock bounds (ms).
#[derive(Debug, Clone)]
pub struct ReplayPolicy {
    pub max_age_ms: u64,
    pub max_future_skew_ms: u64,
}

impl Default for ReplayPolicy {
    fn default() -> Self {
        Self {
            max_age_ms: 300_000,
            max_future_skew_ms: 60_000,
        }
    }
}

/// Canonical length-delimited bytes for HMAC (not JSON field order).
pub fn canonical_signing_bytes(
    event: &InvalidationEvent,
    deployment_id: &str,
    key_id: &str,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(256);
    write_u8(&mut buf, event.protocol_version);
    write_u128(&mut buf, event.event_id);
    write_str(&mut buf, &event.source_node_id);
    write_str(&mut buf, deployment_id);
    write_u64(&mut buf, event.site_id);
    write_str(&mut buf, operation_tag(event.operation));
    match &event.url {
        Some(u) => {
            write_u8(&mut buf, 1);
            write_str(&mut buf, &u.scheme);
            write_str(&mut buf, &u.host);
            write_str(&mut buf, &u.path);
            write_str(&mut buf, &u.query);
        }
        None => write_u8(&mut buf, 0),
    }
    write_u64(&mut buf, event.generation);
    write_u64(&mut buf, event.issued_at_unix_ms);
    write_str(&mut buf, key_id);
    buf
}

fn operation_tag(op: InvalidationOperation) -> &'static str {
    match op {
        InvalidationOperation::PurgeUrl => "purge.url",
        InvalidationOperation::PurgeSite => "purge.site",
        InvalidationOperation::PurgeGeneration => "purge.generation",
    }
}

fn write_u8(buf: &mut Vec<u8>, v: u8) {
    buf.push(v);
}
fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_u128(buf: &mut Vec<u8>, v: u128) {
    buf.extend_from_slice(&v.to_be_bytes());
}
fn write_str(buf: &mut Vec<u8>, s: &str) {
    let len = s.len() as u32;
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(s.as_bytes());
}

pub fn sign_mac(
    event: &InvalidationEvent,
    deployment_id: &str,
    keys: &EventSigningKeys,
) -> Result<(String, String), CoordinationError> {
    let key_id = keys.active_key_id.clone();
    let bytes = canonical_signing_bytes(event, deployment_id, &key_id);
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&keys.active_key)
        .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
    mac.update(&bytes);
    let tag = mac.finalize().into_bytes();
    Ok((key_id, hex_encode(&tag)))
}

pub fn verify_mac(
    event: &InvalidationEvent,
    deployment_id: &str,
    key_id: &str,
    mac_hex: &str,
    keys: &EventSigningKeys,
) -> Result<(), CoordinationError> {
    let Some(key) = keys.key_for_id(key_id) else {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    };
    let expected = hex_decode(mac_hex)
        .ok_or_else(|| CoordinationError::new(CoordinationRejectReason::InvalidTarget))?;
    let bytes = canonical_signing_bytes(event, deployment_id, key_id);
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .map_err(|_| CoordinationError::new(CoordinationRejectReason::InternalError))?;
    mac.update(&bytes);
    mac.verify_slice(&expected)
        .map_err(|_| CoordinationError::new(CoordinationRejectReason::InvalidTarget))?;
    Ok(())
}

pub fn check_issued_at(
    issued_at_unix_ms: u64,
    replay: &ReplayPolicy,
    now_ms: u64,
) -> Result<(), CoordinationError> {
    if issued_at_unix_ms > now_ms.saturating_add(replay.max_future_skew_ms) {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    }
    if now_ms.saturating_sub(issued_at_unix_ms) > replay.max_age_ms {
        return Err(CoordinationError::new(
            CoordinationRejectReason::InvalidTarget,
        ));
    }
    Ok(())
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0xf) as usize] as char);
    }
    s
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let hi = from_hex(bytes[i])?;
        let lo = from_hex(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};

    /// Process env HMAC keys are global — serialize env-mutating tests (KF-P16-010).
    fn hmac_env_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn unique_tmp(tag: &str) -> std::path::PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wc7d_hmac_{tag}_{}_{n}", std::process::id()))
    }
    use exyonq_module_api::{UrlTarget, COORDINATION_PROTOCOL_VERSION};

    fn sample() -> InvalidationEvent {
        InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id: 42,
            source_node_id: "node-a".into(),
            site_id: 1,
            operation: InvalidationOperation::PurgeUrl,
            url: Some(UrlTarget {
                scheme: "https".into(),
                host: "ex.test".into(),
                path: "/x".into(),
                query: String::new(),
            }),
            generation: 3,
            issued_at_unix_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn sign_verify_roundtrip() {
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let ev = sample();
        let (kid, mac) = sign_mac(&ev, "deploy1", &keys).unwrap();
        verify_mac(&ev, "deploy1", &kid, &mac, &keys).unwrap();
        check_issued_at(
            ev.issued_at_unix_ms,
            &ReplayPolicy {
                max_age_ms: u64::MAX / 4,
                max_future_skew_ms: u64::MAX / 4,
            },
            ev.issued_at_unix_ms,
        )
        .unwrap();
    }

    #[test]
    fn mutated_field_fails() {
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let mut ev = sample();
        let (kid, mac) = sign_mac(&ev, "deploy1", &keys).unwrap();
        ev.site_id = 99;
        assert!(verify_mac(&ev, "deploy1", &kid, &mac, &keys,).is_err());
    }

    #[test]
    fn unknown_key_rejected() {
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let ev = sample();
        let (_kid, mac) = sign_mac(&ev, "deploy1", &keys).unwrap();
        assert!(verify_mac(&ev, "deploy1", "k-unknown", &mac, &keys,).is_err());
    }

    #[test]
    fn previous_key_accepted_during_overlap() {
        let keys = EventSigningKeys::for_tests("k2", b"new-secret-key-material-32b!!")
            .with_previous("k1", b"old-secret-key-material-32b!!");
        let ev = sample();
        let old_only = EventSigningKeys::for_tests("k1", b"old-secret-key-material-32b!!");
        let (kid, mac) = sign_mac(&ev, "deploy1", &old_only).unwrap();
        assert_eq!(kid, "k1");
        verify_mac(&ev, "deploy1", &kid, &mac, &keys).unwrap();
    }

    #[test]
    fn expired_event_rejected() {
        let ev = sample();
        let now = ev.issued_at_unix_ms + 400_000;
        assert!(check_issued_at(
            ev.issued_at_unix_ms,
            &ReplayPolicy {
                max_age_ms: 300_000,
                max_future_skew_ms: 60_000,
            },
            now,
        )
        .is_err());
    }

    fn clear_hmac_env() {
        for k in [
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY",
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID",
            "EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY",
            "EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_FILE",
            "EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_ID",
        ] {
            std::env::remove_var(k);
        }
    }

    #[test]
    fn key_file_active_and_previous_load() {
        let _env = hmac_env_test_lock();
        clear_hmac_env();
        let dir = unique_tmp("pair");
        let _ = std::fs::create_dir_all(&dir);
        let active_path = dir.join("active");
        let prev_path = dir.join("previous");
        std::fs::write(&active_path, b"active-file-secret-32bytes!!!!!!").unwrap();
        std::fs::write(&prev_path, b"previous-file-secret-32bytes!!!!").unwrap();

        std::env::set_var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID", "k2");
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            active_path.to_str().unwrap(),
        );
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_ID", "k1");
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_PREVIOUS_KEY_FILE",
            prev_path.to_str().unwrap(),
        );

        let keys = EventSigningKeys::from_env().expect("ACTIVE_KEY_FILE + PREVIOUS_KEY_FILE");
        assert_eq!(keys.active_key, b"active-file-secret-32bytes!!!!!!");
        assert_eq!(
            keys.previous_key.as_deref(),
            Some(b"previous-file-secret-32bytes!!!!".as_slice())
        );
        let dbg = format!("{keys:?}");
        assert!(!dbg.contains("active-file-secret"));
        assert!(!dbg.contains("previous-file-secret"));

        clear_hmac_env();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_empty_and_unreadable_rejected() {
        let _env = hmac_env_test_lock();
        clear_hmac_env();
        let dir = unique_tmp("err");
        let _ = std::fs::create_dir_all(&dir);
        let empty = dir.join("empty");
        std::fs::write(&empty, b"").unwrap();
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID", "k1");
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            empty.to_str().unwrap(),
        );
        assert!(EventSigningKeys::from_env().is_err(), "EMPTY_KEY_FILE");

        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            dir.to_str().unwrap(), // directory
        );
        assert!(EventSigningKeys::from_env().is_err(), "directory FILE");

        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            format!("/tmp/wc7d_missing_hmac_secret_{}", std::process::id()),
        );
        assert!(EventSigningKeys::from_env().is_err(), "UNREADABLE_KEY_FILE");

        clear_hmac_env();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn key_file_wins_over_direct_env() {
        let _env = hmac_env_test_lock();
        clear_hmac_env();
        let path = unique_tmp("conflict");
        std::fs::write(&path, b"from-file-wins-over-env-secret!!!!").unwrap();
        std::env::set_var("EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_ID", "k1");
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY",
            "from-env-must-be-ignored",
        );
        std::env::set_var(
            "EXYONQ_L2_EVENT_HMAC_ACTIVE_KEY_FILE",
            path.to_str().unwrap(),
        );
        let keys = EventSigningKeys::from_env().expect("FILE precedence");
        assert_eq!(keys.active_key, b"from-file-wins-over-env-secret!!!!");
        clear_hmac_env();
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn previous_key_not_used_for_signing() {
        let keys = EventSigningKeys::for_tests("k2", b"new-secret-key-material-32b!!")
            .with_previous("k1", b"old-secret-key-material-32b!!");
        let ev = sample();
        let (kid, _) = sign_mac(&ev, "deploy1", &keys).unwrap();
        assert_eq!(kid, "k2", "PREVIOUS_KEY_USED_FOR_SIGNING=NO");
    }

    /// Independent known-answer vector (RustCrypto hmac 0.13 docs / standard HMAC-SHA256).
    /// Proves library digest+tag bytes, not merely self-sign/self-verify.
    #[test]
    fn hmac_sha256_known_answer_vector() {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(b"my secret and secure key")
            .expect("HMAC accepts arbitrary key length");
        mac.update(b"input message");
        let tag = mac.finalize().into_bytes();
        let expected =
            hex_decode("97d2a569059bbcd8ead4444ff99071f4c01d005bcefe0d3567e1be628e5fdcd9")
                .expect("public hex vector");
        assert_eq!(
            tag.as_slice(),
            expected.as_slice(),
            "HMAC_SHA256_KNOWN_ANSWER"
        );

        let mut v = <HmacSha256 as KeyInit>::new_from_slice(b"my secret and secure key").unwrap();
        v.update(b"input message");
        v.verify_slice(&expected).expect("known vector verifies");
    }

    #[test]
    fn modified_tag_rejected() {
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let ev = sample();
        let (kid, mac) = sign_mac(&ev, "deploy1", &keys).unwrap();
        let mut bad = mac.into_bytes();
        // Flip one nibble without changing length / hex validity.
        let b0 = bad[0];
        bad[0] = if b0 == b'0' { b'1' } else { b'0' };
        let bad = String::from_utf8(bad).unwrap();
        assert!(verify_mac(&ev, "deploy1", &kid, &bad, &keys).is_err());
    }

    #[test]
    fn wrong_key_rejects_valid_tag() {
        let signer = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let other = EventSigningKeys::for_tests("k1", b"other-key-material-32bytes!!!!");
        let ev = sample();
        let (kid, mac) = sign_mac(&ev, "deploy1", &signer).unwrap();
        assert!(verify_mac(&ev, "deploy1", &kid, &mac, &other).is_err());
    }

    #[test]
    fn truncated_and_malformed_tag_rejected() {
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let ev = sample();
        let (kid, mac) = sign_mac(&ev, "deploy1", &keys).unwrap();
        assert!(
            verify_mac(&ev, "deploy1", &kid, &mac[..mac.len() - 2], &keys).is_err(),
            "truncated tag must fail (no truncation contract)"
        );
        assert!(
            verify_mac(&ev, "deploy1", &kid, "zz", &keys).is_err(),
            "malformed hex must fail"
        );
        assert!(
            verify_mac(&ev, "deploy1", &kid, "abc", &keys).is_err(),
            "odd-length hex must fail"
        );
    }
}
