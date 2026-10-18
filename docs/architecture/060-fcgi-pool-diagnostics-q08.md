# ADR-060 — FastCGI pool saturation diagnostics (Q-08 / v0.6 F2)

**Status:** Accepted
**Date:** 2026-10-18
**Depends on:** Cap FastCGI ConnPool, `fcgi-saturation-policy-v1.md`, metrics composition root
**Contract:** CONDITIONAL ACCEPT — actionable log on pool busy + `exyonq_fcgi_*` on `/metrics`

## Context

QUEJAS_OLS §2.6: operators see opaque LSAPI deadlock messages with no timeout, socket, or remediation hint. ExyonQ already maps saturation to HTTP 503 (`PoolBusy`) and keeps process-wide counters, but those counters were not exported as `exyonq_fcgi_*` and busy paths did not log a structured recommendation.

## Decision

1. **Saturation log (warn):** when checkout returns `PoolBusy` (checkout wait exhausted, drain/closed, or capacity wait cancelled), emit one structured warn with:
   - `pool` (config name when known, else `pool_<id>` / `unnamed`)
   - `timeout_kind` ∈ `{checkout, drain, capacity, closed}`
   - `open_sockets`, `idle_sockets`, `waiters`, `max_connections`
   - `endpoint` (unix path or tcp host:port)
   - `recommendation` (operator next step)
2. **Prometheus:** `append_fcgi_prometheus` registers process-wide gauges/counters under the `exyonq_fcgi_*` prefix (responses, saturation, checkout timeouts, inflight, open/idle via last pool ops snapshot fields already tracked).
3. **Per-pool labels (v1):** log carries the pool name; metric series remain **process aggregates** (same as existing atomics) to avoid unbounded cardinality. Named series with `pool=` labels are a follow-up if multi-pool production evidence requires it.
4. **Non-goals:** changing 503 mapping, unbounded wait queues, LSAPI watchdog messages.

## Consequences

- Saturating a pool produces a grep-friendly warn and scrapable `exyonq_fcgi_*` series.
- Operators raise `max_concurrency` / `checkout_timeout_ms` or fix PHP-FPM using the recommendation text.
