//! Cap061 production observability leaf crate.
//!
//! Composition root (`cli/exyonq`) installs subscribers. `core` emits tracing
//! events only — no OpenTelemetry types cross the core public boundary.

mod access;
mod bounds;
mod correlation;
mod install;
mod redaction;
#[cfg(feature = "file")]
mod size_rotate;

#[cfg(all(feature = "journald", target_os = "linux"))]
mod journald_sink;
#[cfg(feature = "otel")]
mod otel;
#[cfg(feature = "syslog")]
mod syslog_sink;

pub use access::{emit_access, AccessEvent};
pub use bounds::{
    file_drop_count, file_nonblocking_bounds, journald_bounds, otel_batch_bounds, sink_error_count,
    syslog_bounds, BoundReport,
};
pub use correlation::{generate_internal_request_id, resolve_request_identity, RequestIdentity};
pub use install::{
    bootstrap_from_env, install_from_config, reload_from_config, reloadability_matrix,
    ObservabilityGuard,
};
pub use redaction::{
    header_value_for_log, is_sensitive_header_name, redact_header_pair, scrub_field_blob, REDACTED,
};

/// Cap061 E2E / self-test: emit a sensitive-shaped field through the live subscriber.
/// Fanout (+ journald native path) must scrub before any sink observes the sentinel.
/// Cap061 E2E / self-test: emit a sensitive-shaped value through the live subscriber.
///
/// **Emit-site scrub (OTel-safe):** the value is redacted *before* `tracing` records
/// it so Fanout sinks *and* OpenTelemetryLayer never observe cleartext sentinels
/// (SEC-CAP061-OTLP-REDACTION). Fanout still applies defense-in-depth `scrub_field_blob`.
pub fn emit_redaction_probe(sentinel: &str) {
    let raw = format!("{}{}", "Bearer ", sentinel);
    let scrubbed = crate::redaction::scrub_field_blob(&raw);
    tracing::info!(
        event = "cap061_redaction_probe",
        Authorization = %scrubbed,
        "cap061_redaction_probe"
    );
}
