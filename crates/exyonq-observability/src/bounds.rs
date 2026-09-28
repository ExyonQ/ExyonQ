//! Cap061 resource-bound inventory (file/OTel/syslog).

use std::sync::atomic::{AtomicU64, Ordering};

/// Process-wide count of dropped log lines from bounded non-blocking writers.
static FILE_DROP_COUNT: AtomicU64 = AtomicU64::new(0);
static SINK_ERROR_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn note_file_drops(n: u64) {
    if n > 0 {
        FILE_DROP_COUNT.fetch_add(n, Ordering::Relaxed);
    }
}

pub fn note_sink_error() {
    SINK_ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn file_drop_count() -> u64 {
    FILE_DROP_COUNT.load(Ordering::Relaxed)
}

pub fn sink_error_count() -> u64 {
    SINK_ERROR_COUNT.load(Ordering::Relaxed)
}

/// Documented Cap061 queue/backpressure contract (audited against configured sinks).
#[derive(Debug, Clone, Copy)]
pub struct BoundReport {
    pub queue_type: &'static str,
    pub queue_bound: u32,
    pub drop_policy: &'static str,
    pub drop_accounting: &'static str,
    pub blocking_behavior: &'static str,
    pub retry_policy: &'static str,
    pub retry_bound: &'static str,
    pub shutdown_flush_bound: &'static str,
}

pub fn file_nonblocking_bounds(queue_capacity: u32) -> BoundReport {
    BoundReport {
        queue_type: "crossbeam_bounded_channel_tracing_appender",
        queue_bound: queue_capacity,
        drop_policy: "drop_newest_when_full",
        drop_accounting: "NonBlocking::error_counter + FILE_DROP_COUNT",
        blocking_behavior: "request_task_never_blocks_on_file_worker",
        retry_policy: "none",
        retry_bound: "0",
        shutdown_flush_bound: "WorkerGuard_drop_flushes_best_effort",
    }
}

pub fn otel_batch_bounds() -> BoundReport {
    BoundReport {
        queue_type: "opentelemetry_sdk_batch_span_processor",
        queue_bound: 2048, // SDK default batch queue (documented; not unbounded)
        drop_policy: "sdk_batch_drop_when_full",
        drop_accounting: "otel_sdk_internal_plus_SINK_ERROR_COUNT_on_export_fail",
        blocking_behavior: "export_on_batch_worker_not_request_task",
        retry_policy: "sdk_bounded_export_retries",
        retry_bound: "sdk_default_finite",
        shutdown_flush_bound: "force_flush_then_SdkTracerProvider::shutdown",
    }
}

pub fn syslog_bounds() -> BoundReport {
    BoundReport {
        queue_type: "none_direct_write",
        queue_bound: 0,
        drop_policy: "drop_on_io_error_best_effort",
        drop_accounting: "SINK_ERROR_COUNT_when_emit_fails",
        blocking_behavior: "may_block_briefly_on_syslog_socket_write",
        retry_policy: "none",
        retry_bound: "0",
        shutdown_flush_bound: "socket_drop_on_generation_swap",
    }
}

pub fn journald_bounds() -> BoundReport {
    BoundReport {
        queue_type: "none_native_sd_journal_send",
        queue_bound: 0,
        drop_policy: "drop_on_journal_send_error",
        drop_accounting: "fanout_journald_datagram_errors_via_note_sink_error",
        blocking_behavior: "may_block_briefly_on_journal_socket",
        retry_policy: "none",
        retry_bound: "0",
        shutdown_flush_bound: "layer_drop_on_reload",
    }
}
