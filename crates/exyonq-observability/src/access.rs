//! Canonical access-event emission (structured tracing fields).

use tracing::info;

/// Final observable access event — emit only after terminal status is known.
#[derive(Debug, Clone)]
pub struct AccessEvent<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub status: u16,
    pub protocol: &'a str,
    pub duration_ms: Option<u64>,
    pub bytes_sent: Option<u64>,
    pub client_ip: Option<&'a str>,
    pub external_request_id: Option<&'a str>,
    pub internal_request_id: &'a str,
    pub upstream: Option<&'a str>,
    pub error_class: Option<&'a str>,
    pub outcome: Option<&'a str>,
}

pub fn emit_access(ev: AccessEvent<'_>) {
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
