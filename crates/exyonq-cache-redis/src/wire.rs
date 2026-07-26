//! Versioned signed wire envelope (HMAC over canonical bytes, not JSON order).

use crate::auth::{
    check_issued_at, now_unix_ms, sign_mac, verify_mac, EventSigningKeys, ReplayPolicy,
};
use exyonq_module_api::{
    validate_invalidation_event, CoordinationError, CoordinationRejectReason, InvalidationEvent,
};
use serde::{Deserialize, Serialize};

/// Wire envelope: event JSON for transport; MAC over canonical binary.
#[derive(Debug, Serialize, Deserialize)]
pub struct SignedWireEnvelope {
    pub key_id: String,
    pub mac: String,
    pub event: InvalidationEvent,
}

pub fn encode_signed_event(
    event: &InvalidationEvent,
    deployment_id: &str,
    keys: &EventSigningKeys,
    max_bytes: usize,
) -> Result<String, CoordinationError> {
    validate_invalidation_event(event, max_bytes)?;
    let (key_id, mac) = sign_mac(event, deployment_id, keys)?;
    let env = SignedWireEnvelope {
        key_id,
        mac,
        event: event.clone(),
    };
    let s = serde_json::to_string(&env).map_err(|_| {
        CoordinationError::new(CoordinationRejectReason::InternalError)
    })?;
    if s.len() > max_bytes {
        return Err(CoordinationError::new(CoordinationRejectReason::OversizedEvent));
    }
    Ok(s)
}

pub fn decode_signed_event(
    raw: &str,
    deployment_id: &str,
    keys: &EventSigningKeys,
    replay: &ReplayPolicy,
    max_bytes: usize,
) -> Result<InvalidationEvent, CoordinationError> {
    if raw.len() > max_bytes {
        return Err(CoordinationError::new(CoordinationRejectReason::OversizedEvent));
    }
    let env: SignedWireEnvelope = serde_json::from_str(raw).map_err(|_| {
        CoordinationError::new(CoordinationRejectReason::InvalidTarget)
    })?;
    if env.key_id.is_empty() || env.mac.is_empty() {
        return Err(CoordinationError::new(CoordinationRejectReason::InvalidTarget));
    }
    if let Err(e) = check_issued_at(env.event.issued_at_unix_ms, replay, now_unix_ms()) {
        crate::metrics::note_replay_reject();
        return Err(e);
    }
    if let Err(e) = verify_mac(
        &env.event,
        deployment_id,
        &env.key_id,
        &env.mac,
        keys,
    ) {
        crate::metrics::note_hmac_reject();
        return Err(e);
    }
    validate_invalidation_event(&env.event, max_bytes)?;
    Ok(env.event)
}

/// Unsigned decode for migration tests only — production path must use signed.
#[cfg(test)]
pub fn decode_event_unsigned_for_tests(
    raw: &str,
    max_bytes: usize,
) -> Result<InvalidationEvent, CoordinationError> {
    if raw.len() > max_bytes {
        return Err(CoordinationError::new(CoordinationRejectReason::OversizedEvent));
    }
    let event: InvalidationEvent = serde_json::from_str(raw).map_err(|_| {
        CoordinationError::new(CoordinationRejectReason::InvalidTarget)
    })?;
    validate_invalidation_event(&event, max_bytes)?;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{sign_mac, EventSigningKeys, ReplayPolicy};
    use crate::metrics::{
        l2_redis_hmac_reject_total, l2_redis_replay_reject_total, lock_redis_metrics_for_tests,
        reset_redis_metrics_for_tests,
    };
    use exyonq_module_api::{
        InvalidationOperation, UrlTarget, COORDINATION_PROTOCOL_VERSION,
    };

    fn sample() -> InvalidationEvent {
        InvalidationEvent {
            protocol_version: COORDINATION_PROTOCOL_VERSION,
            event_id: 7,
            source_node_id: "n".into(),
            site_id: 1,
            operation: InvalidationOperation::PurgeUrl,
            url: Some(UrlTarget {
                scheme: "https".into(),
                host: "t".into(),
                path: "/".into(),
                query: String::new(),
            }),
            generation: 1,
            issued_at_unix_ms: crate::auth::now_unix_ms(),
        }
    }

    #[test]
    fn hmac_reject_counted_once_in_decode() {
        // KF-P16-010: process-wide reject counters — serialize reset→assert.
        let _metrics = lock_redis_metrics_for_tests();
        reset_redis_metrics_for_tests();
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let ev = sample();
        let (kid, mut mac) = sign_mac(&ev, "d", &keys).unwrap();
        // Corrupt MAC
        mac.push('0');
        let env = SignedWireEnvelope {
            key_id: kid,
            mac,
            event: ev,
        };
        let raw = serde_json::to_string(&env).unwrap();
        let before = l2_redis_hmac_reject_total();
        assert!(decode_signed_event(
            &raw,
            "d",
            &keys,
            &ReplayPolicy::default(),
            8192,
        )
        .is_err());
        assert_eq!(
            l2_redis_hmac_reject_total(),
            before + 1,
            "one invalid event → one hmac reject"
        );
    }

    #[test]
    fn replay_reject_counted_once_in_decode() {
        let _metrics = lock_redis_metrics_for_tests();
        reset_redis_metrics_for_tests();
        let keys = EventSigningKeys::for_tests("k1", b"secret-key-material-32bytes!!");
        let mut ev = sample();
        ev.issued_at_unix_ms = 1; // ancient
        let (kid, mac) = sign_mac(&ev, "d", &keys).unwrap();
        let env = SignedWireEnvelope {
            key_id: kid,
            mac,
            event: ev,
        };
        let raw = serde_json::to_string(&env).unwrap();
        let before = l2_redis_replay_reject_total();
        let before_hmac = l2_redis_hmac_reject_total();
        assert!(decode_signed_event(
            &raw,
            "d",
            &keys,
            &ReplayPolicy {
                max_age_ms: 1_000,
                max_future_skew_ms: 60_000,
            },
            8192,
        )
        .is_err());
        assert_eq!(l2_redis_replay_reject_total(), before + 1);
        assert_eq!(
            l2_redis_hmac_reject_total(),
            before_hmac,
            "replay must not also bump hmac"
        );
    }

    #[test]
    fn unsigned_decode_helper_for_migration_tests() {
        let ev = sample();
        let raw = serde_json::to_string(&ev).unwrap();
        let out = decode_event_unsigned_for_tests(&raw, 8192).unwrap();
        assert_eq!(out.event_id, ev.event_id);
    }
}
