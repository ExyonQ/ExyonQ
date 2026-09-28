//! Cap061 — thin core observability events (no sink/OTel types).
//!
//! Leaf crate `exyonq-observability` owns subscribers/sinks. Core only emits
//! structured `tracing` fields that those sinks format/export.
//!
//! # Multi-event protocol semantics (documented)
//!
//! - **WebSocket**: terminal access event at upgrade response (status 101,
//!   outcome=`websocket_upgrade`). A second optional event with
//!   outcome=`websocket_closed` may be emitted when the tunnel ends if the
//!   proxy path surfaces a close — not required for every silent peer drop.
//! - **SSE**: one terminal access event when response headers are committed
//!   (stream may continue). outcome=`sse` when Content-Type is event-stream.
//! - **HTTP/2 / HTTP/3**: same one-request → one-internal-id → one terminal
//!   access event model as HTTP/1.1 after final status is known.

use exyonq_config_ir::LoggingConfig;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tracing::{info, warn, Instrument};

static ACCESS_LOGGING_ENABLED: AtomicBool = AtomicBool::new(true);
static AUDIT_LOGGING_ENABLED: AtomicBool = AtomicBool::new(true);
static OTEL_SPANS_ENABLED: AtomicBool = AtomicBool::new(false);
/// Cached `EXYONQ_ACCESS_LOG` override: 0 = unresolved, 1 = none, 2 = force off, 3 = force on.
/// Mid-process env mutation is not observed — restart required (S1/S2 lifecycle model).
static ENV_ACCESS_OVERRIDE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

type LoggingReloadHook = Box<dyn Fn(&LoggingConfig) -> Result<(), String> + Send + Sync>;

static LOGGING_RELOAD_HOOK: OnceLock<Mutex<Option<LoggingReloadHook>>> = OnceLock::new();

/// Composition-root registration (cli/exyonq). Core calls this during reload COMMIT.
pub fn register_logging_reload_hook<F>(f: F)
where
    F: Fn(&LoggingConfig) -> Result<(), String> + Send + Sync + 'static,
{
    let cell = LOGGING_RELOAD_HOOK.get_or_init(|| Mutex::new(None));
    *cell.lock().expect("logging reload hook lock") = Some(Box::new(f));
}

/// Apply `[logging]` after ServerState publish (KEEP_OLD on Err).
///
/// Fail-closed when the composition-root hook is unset: product reload must not
/// advance runtime config.logging while sinks stay on bootstrap generation.
pub fn apply_logging_reload(cfg: &LoggingConfig) -> Result<(), String> {
    let Some(cell) = LOGGING_RELOAD_HOOK.get() else {
        return Err("logging reload hook not registered".into());
    };
    let guard = cell
        .lock()
        .map_err(|_| "logging reload hook poisoned".to_string())?;
    match *guard {
        Some(ref hook) => hook(cfg),
        None => Err("logging reload hook not installed".into()),
    }
}

/// Process-wide access logging switch (IR `[logging.access]` + cached env override).
pub fn set_access_logging_enabled(enabled: bool) {
    ACCESS_LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
    // Keep static-wire notices in lockstep with the effective access gate (LA-S2-003).
    exyonq_module_api::static_wire::set_access_notices_enabled(access_event_required());
}

pub fn set_audit_logging_enabled(enabled: bool) {
    AUDIT_LOGGING_ENABLED.store(enabled, Ordering::Relaxed);
}

/// Process-wide OTel request-span switch (IR `[logging.otel]`).
pub fn set_otel_spans_enabled(enabled: bool) {
    OTEL_SPANS_ENABLED.store(enabled, Ordering::Relaxed);
    exyonq_module_api::static_wire::set_wire_otel_spans_enabled(enabled);
}

fn env_access_override() -> u8 {
    let cached = ENV_ACCESS_OVERRIDE.load(Ordering::Relaxed);
    if cached != 0 {
        return cached;
    }
    let resolved = match std::env::var("EXYONQ_ACCESS_LOG") {
        Ok(v) if v == "0" || v.eq_ignore_ascii_case("false") => 2,
        Ok(v) if v == "1" || v.eq_ignore_ascii_case("true") => 3,
        _ => 1,
    };
    let _ = ENV_ACCESS_OVERRIDE.compare_exchange(0, resolved, Ordering::Relaxed, Ordering::Relaxed);
    ENV_ACCESS_OVERRIDE.load(Ordering::Relaxed)
}

pub fn access_logging_enabled() -> bool {
    match env_access_override() {
        2 => false,
        3 => true,
        _ => ACCESS_LOGGING_ENABLED.load(Ordering::Relaxed),
    }
}

pub fn audit_logging_enabled() -> bool {
    AUDIT_LOGGING_ENABLED.load(Ordering::Relaxed)
}

pub fn otel_spans_enabled() -> bool {
    OTEL_SPANS_ENABLED.load(Ordering::Relaxed)
}

/// Access terminal / wire access-event construction required.
#[inline]
pub fn access_event_required() -> bool {
    access_logging_enabled()
}

/// Internal request-id generation / response echo / OTel span identity required.
#[inline]
pub fn request_id_required() -> bool {
    access_event_required() || otel_spans_enabled()
}

/// EXTERNAL vs INTERNAL request identity (Cap061).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestIdentity {
    /// Validated client `x-request-id` when acceptable; never a security identity.
    pub external_request_id: Option<String>,
    /// Always server-generated; used for internal correlation and response echo.
    pub internal_request_id: String,
}

pub fn generate_internal_request_id() -> String {
    let mut buf = [0u8; 16];
    // Prefer OS entropy; fall back to time+counter if unavailable.
    if getrandom_fill(&mut buf).is_err() {
        use std::sync::atomic::{AtomicU64, Ordering as Ord};
        use std::time::{SystemTime, UNIX_EPOCH};
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let n = COUNTER.fetch_add(1, Ord::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0);
        buf[..8].copy_from_slice(&nanos.to_le_bytes());
        buf[8..].copy_from_slice(&n.to_le_bytes());
    }
    let mut out = String::with_capacity(32);
    for b in buf {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn getrandom_fill(buf: &mut [u8]) -> Result<(), ()> {
    // Avoid new core dependency: use libc getentropy / Mac getentropy via std when possible.
    // `std::sys` is private; use file read of /dev/urandom on unix as portable fallback.
    #[cfg(unix)]
    {
        use std::fs::File;
        use std::io::Read;
        let mut f = File::open("/dev/urandom").map_err(|_| ())?;
        f.read_exact(buf).map_err(|_| ())
    }
    #[cfg(not(unix))]
    {
        let _ = buf;
        Err(())
    }
}

/// Validate external `x-request-id`.
///
/// Contract:
/// - missing → None (generate internal only)
/// - empty / whitespace → rejected (None)
/// - >128 bytes → rejected
/// - non-allowed charset → rejected
/// - duplicate headers: caller must pass the first value only (http::HeaderMap::get)
/// - never copied into INTERNAL_REQUEST_ID
pub fn valid_external_request_id(raw: &str) -> Option<String> {
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

pub fn resolve_request_identity(raw_x_request_id: Option<&str>) -> RequestIdentity {
    RequestIdentity {
        external_request_id: raw_x_request_id.and_then(valid_external_request_id),
        internal_request_id: generate_internal_request_id(),
    }
}

/// Terminal HTTP access event — emit only after final observable status is known.
#[derive(Debug, Clone)]
pub struct AccessTerminal<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub status: u16,
    pub protocol: &'a str,
    pub duration_ms: u64,
    pub bytes_sent: Option<u64>,
    pub client_ip: Option<&'a str>,
    pub external_request_id: Option<&'a str>,
    pub internal_request_id: &'a str,
    pub upstream: Option<&'a str>,
    pub error_class: Option<&'a str>,
    /// e.g. ok | waf_block | waf_challenge | rate_limited | cache_hit | fpc_hit | draining | probe
    pub outcome: Option<&'a str>,
}

pub fn emit_access_terminal(ev: AccessTerminal<'_>) {
    if !access_event_required() {
        return;
    }
    info!(
        event = "access",
        method = %ev.method,
        path = %ev.path,
        status = ev.status,
        protocol = %ev.protocol,
        duration_ms = ev.duration_ms,
        bytes_sent = ev.bytes_sent,
        client_ip = ev.client_ip,
        external_request_id = ev.external_request_id,
        request_id = %ev.internal_request_id,
        upstream = ev.upstream,
        error_class = ev.error_class,
        outcome = ev.outcome,
        "access"
    );
}

/// Security/operational audit — only after STATE_CHANGE_COMMITTED or definitive failure.
#[derive(Debug, Clone)]
pub struct AuditEvent<'a> {
    pub action: &'a str,
    pub result: &'a str, // success | failure
    pub detail: Option<&'a str>,
    pub request_id: Option<&'a str>,
}

pub fn emit_audit(ev: AuditEvent<'_>) {
    if !audit_logging_enabled() {
        return;
    }
    if ev.result == "success" {
        info!(
            event = "audit",
            action = %ev.action,
            result = %ev.result,
            detail = ev.detail,
            request_id = ev.request_id,
            "audit"
        );
    } else {
        warn!(
            event = "audit",
            action = %ev.action,
            result = %ev.result,
            detail = ev.detail,
            request_id = ev.request_id,
            "audit"
        );
    }
}

/// Optional hint attached to a response before terminal access emit.
/// Used when status alone cannot distinguish outcomes (e.g. FPC hit vs origin 200).
#[derive(Clone, Copy, Debug)]
pub struct AccessOutcomeHint(pub &'static str);

pub fn attach_outcome_hint<B>(
    mut response: http::Response<B>,
    hint: &'static str,
) -> http::Response<B> {
    response.extensions_mut().insert(AccessOutcomeHint(hint));
    response
}

pub fn outcome_hint<B>(response: &http::Response<B>) -> Option<&'static str> {
    response
        .extensions()
        .get::<AccessOutcomeHint>()
        .map(|h| h.0)
}

/// Open a request span for OTel correlation (leaf OpenTelemetryLayer maps this).
pub fn request_span(
    identity: &RequestIdentity,
    method: &str,
    path: &str,
    protocol: &str,
) -> tracing::Span {
    tracing::info_span!(
        "request",
        request_id = %identity.internal_request_id,
        external_request_id = identity.external_request_id.as_deref().unwrap_or(""),
        method = %method,
        path = %path,
        protocol = %protocol,
        otel.kind = "server",
    )
}

pub async fn in_request_span<F, T>(span: tracing::Span, fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    fut.instrument(span).await
}

pub fn duration_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_control_and_oversize() {
        assert!(valid_external_request_id("bad\nid").is_none());
        assert!(valid_external_request_id(&"a".repeat(129)).is_none());
        assert_eq!(
            valid_external_request_id("req-abc_01").as_deref(),
            Some("req-abc_01")
        );
    }

    #[test]
    fn internal_never_equals_untrusted_external_copy() {
        let id = resolve_request_identity(Some("client-supplied"));
        assert_eq!(id.external_request_id.as_deref(), Some("client-supplied"));
        assert_ne!(id.internal_request_id, "client-supplied");
        assert_eq!(id.internal_request_id.len(), 32);
    }

    #[test]
    fn request_id_required_follows_access_and_otel_flags() {
        let prev_access = access_logging_enabled();
        let prev_otel = otel_spans_enabled();
        set_access_logging_enabled(false);
        set_otel_spans_enabled(false);
        // Clear any EXYONQ_ACCESS_LOG force-on from the process environment for this assertion.
        // Env override is process-lifetime; if force-on is set externally, skip.
        if env_access_override() == 3 {
            set_access_logging_enabled(prev_access);
            set_otel_spans_enabled(prev_otel);
            return;
        }
        assert!(!access_event_required());
        assert!(!request_id_required());
        set_otel_spans_enabled(true);
        assert!(request_id_required());
        assert!(!access_event_required());
        set_otel_spans_enabled(false);
        set_access_logging_enabled(true);
        if env_access_override() != 2 {
            assert!(access_event_required());
            assert!(request_id_required());
        }
        set_access_logging_enabled(prev_access);
        set_otel_spans_enabled(prev_otel);
    }

    #[test]
    fn emit_access_terminal_skips_when_access_disabled() {
        let prev = access_logging_enabled();
        if env_access_override() == 3 {
            return;
        }
        set_access_logging_enabled(false);
        // Must not panic / allocate identity — empty id is only valid because gate returns first.
        emit_access_terminal(AccessTerminal {
            method: "GET",
            path: "/",
            status: 200,
            protocol: "test",
            duration_ms: 0,
            bytes_sent: None,
            client_ip: None,
            external_request_id: None,
            internal_request_id: "",
            upstream: None,
            error_class: None,
            outcome: Some("ok"),
        });
        set_access_logging_enabled(prev);
    }
}
