# ADR 037 — Production Observability Architecture (Capability 061)

```text
ADR = 037
TITLE = Production Observability Architecture
STATUS = ACCEPTED
DATE = 2026-08-11
CAPABILITY_ID = 061
FEATURE_ID = observability-logging
OWNER_ACCEPTANCE = ACCEPT
IMPLEMENTATION_AUTHORIZED = YES
CAP061_OWNER_SCOPE_EXPANDED = YES
CAP061_FULL_OBSERVABILITY_REQUIRED = YES
```

## Context

Historical Cap061 matrix scope was `tracing` + JSON via `EXYONQ_LOG_FORMAT`. That
records implementation history only. The owner expanded Cap061 to a complete
production observability subsystem:

```text
HISTORICAL_CAP061_CONTRACT = tracing + JSON log format
CURRENT_OWNER_REQUIRED_CONTRACT = FULL_PRODUCTION_OBSERVABILITY
CAP061_INITIAL_ASSESSMENT = IMPLEMENTATION_PARTIAL_NEEDS_PRODUCT_FIX
MISSING_REQUIRED_PRODUCT_CAPABILITY = YES
PRIMARY_REALITY_STATUS_BEFORE = IMPLEMENTED_NOT_REAL_E2E_PROVEN
```

RD-005: `packaging/config/config.toml.example` already documents `[logging]`.
`RawConfig` previously rejected it (`deny_unknown_fields`). Classification:

```text
RD-005_CLASSIFICATION =
MISSING_REQUIRED_IMPLEMENTATION_BEHIND_EXISTING_PRODUCT_CONFIG
```

Implement the IR; do not delete the packaging claim.

Cap054 Prometheus remains administratively closed and is the metrics scrape
authority. Cap061 must preserve it and prove coherence. LA-CAP054-008 was
resolved globally (rate-limit before metrics scrape; `scrape_bearer_token`
required for non-loopback binds) without reopening Cap054.

## Decisions

1. **Leaf crate** `crates/exyonq-observability` owns sinks, subscriber composition,
   OTel export, redaction helpers, and generation-scoped runtime state.
2. **Composition root** is `cli/exyonq`. It bootstraps from env, then applies
   IR `[logging]` on `serve` (and reload prepare/commit when supported).
3. **`core/` emits tracing events only** — no `opentelemetry::*` types on
   `core` / `module-api` public surfaces.
4. **Canonical IR section** is `[logging]` (packaging name). Alias
   `[observability]` is not required in v1 if `[logging]` is complete.
5. **Formatters**: text and JSON (genuine structured fields, not a single
   preformatted string). Destinations: stdout/stderr, file (+ rotation),
   syslog, journald (native structured on systemd Linux), OTLP.
6. **Metrics authority**: Cap054 Prometheus path remains authoritative.
   OpenTelemetry metrics export is optional / non-authoritative by default
   unless a later ADR revises this after evaluation.
7. **Correlation**: separate `EXTERNAL_REQUEST_ID` (validated client header)
   from `INTERNAL_REQUEST_ID` / trace identity. Do not treat arbitrary client
   input as a security identity.
8. **Access logging**: INFO-level structured `event="access"` after the final
   observable status is known (not DEBUG-gated).
9. **Reload**: prepare/validate/commit for observability sinks. Failure keeps
   the previous generation. Restart-only knobs must be documented exactly.
10. **Backpressure**: bounded queues; drop accounting when lossy; no data-plane
    deadlock on slow sinks; no process panic solely because a log file fails
    unless STARTUP_FATAL is explicit.
11. **Zero-fake**: terminal proof uses real OTel Collector, real syslog/journald,
    real filesystem. No in-memory exporter as terminal evidence.
12. **Dependencies**: Cap061 may add maintained crates (`tracing-appender`,
    OpenTelemetry stack, syslog, `tracing-journald`) under
    `DEPENDENCY_CHANGE_AUTHORIZED=YES_FOR_CAP061_REQUIRED_IMPLEMENTATION`,
    then requalify the lock baseline.

## Consequences

- Cargo.lock will change; historical dependency seal remains historical.
- Dual-Linux Cap061 harness required (owner matrix A–Z), not Cap051 incidental
  `log_format_json_env`.
- Cap062 / Cap067 remain `STARTED=NO`. Phase 1 stays open until Cap061 closes
  under the expanded contract.
