# ADR-060 — FastCGI pool saturation diagnostics (Q-08 / v0.6 F2)

**Status:** Accepted — F6-WP-02 reconfirmed 2026-11-02
**Date:** 2026-10-18  
**Depends on:** Cap FastCGI ConnPool, `fcgi-saturation-policy-v1.md`, metrics composition root  
**Contract:** CONDITIONAL ACCEPT — actionable log on pool busy/concurrency + `exyonq_fcgi_*` global and `exyonq_fcgi_pool_*{pool=…}` on `/metrics`

## Context

QUEJAS_OLS §2.6: operators see opaque LSAPI deadlock messages with no timeout, socket, or remediation hint. ExyonQ already maps saturation to HTTP 503 (`PoolBusy`) and keeps process-wide counters, but those counters were not exported as `exyonq_fcgi_*` and busy paths did not log a structured recommendation.

## Decision

1. **Saturation log (warn):** when checkout returns `PoolBusy` (checkout wait exhausted, drain/closed, or capacity wait cancelled), or the logical `max_concurrency` semaphore rejects, emit one structured warn with:
   - `pool` (config name when known, else `pool_<id>` / `unnamed`)
   - `timeout_kind` ∈ `{checkout, drain, capacity, concurrency, closed}`
   - `queue=disabled` in the message
   - `max_concurrency` on the logical-concurrency path
   - `open_sockets`, `idle_sockets`, `waiters`, `max_connections` on the
     `PoolBusy`/checkout path
   - `endpoint` (unix path or tcp host:port) on the `PoolBusy` path
   - `recommendation` (operator next step)
2. **Prometheus:** `append_fcgi_prometheus` keeps the process-wide `exyonq_fcgi_*` series for compatibility and now exports per-pool `exyonq_fcgi_pool_responses_total{pool,status}`, `exyonq_fcgi_pool_saturation_rejections_total{pool}`, `exyonq_fcgi_pool_checkout_timeouts_total{pool}` and `exyonq_fcgi_pool_inflight_current{pool}`. Pool labels come only from configured pool names.
3. **Cardinality:** the named-pool registry is capped at 64 configured pools; excess names share `pool="__overflow__"`. This is a config-defined bound, not a per-request label. The global series remain the fallback for existing dashboards.
4. **Non-goals:** changing 503 mapping, unbounded wait queues, LSAPI watchdog messages.

## Consequences

- Saturating a pool produces a grep-friendly warn and scrapable `exyonq_fcgi_*` series.  
- Operators raise `max_concurrency` / `checkout_timeout_ms` or fix PHP-FPM using the recommendation text.
