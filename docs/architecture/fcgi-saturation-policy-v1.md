# FastCGI saturation policy (Phase 1 / C2)

**Status:** Accepted — F6-WP-02 decision, 2026-11-02
**Owner crate:** `crates/exyonq-mod-fastcgi/src/conn_pool.rs`

## Decision: bounded fail-fast, no unbounded queue

ExyonQ does **not** implement an unbounded NGINX-style FastCGI wait queue.
The F6-WP-02 decision is to **keep the existing fail-fast mapping** rather than
add a product-managed queue: the queue would move the same capacity limit into
resident waiters without creating PHP-FPM workers, and would make memory and
latency under adversarial load harder to bound. The bounded checkout wait is
not an NGINX queue and must not be described as one.

When the pool has no idle connection and cannot open another under
`max_connections` / `max_concurrency`:

1. The request may wait up to `checkout_timeout` (IR: `checkout_timeout_ms`).
2. If checkout wait is exhausted (or the absolute request budget expires during
   checkout), the pool returns `WireError::PoolBusy`.
3. The data plane maps that to **HTTP 503** (pool busy), not an infinite queue.

## Caps

| Cap | Role |
| --- | --- |
| `max_concurrency` | Concurrent in-flight FastCGI requests admitted by the pool |
| `max_connections` | Open backend sockets retained/opened by the pool (defaults with concurrency when unset) |
| `checkout_timeout_ms` | Bounded waiter budget before `PoolBusy` |

## Immediate reject

Saturation rejection is immediate when the pool is closed, `max_connections == 0`,
or capacity is exhausted with no path to obtain a connection before the checkout
deadline. The logical `max_concurrency` semaphore rejects immediately when all
permits are held; that path returns HTTP 503 and now emits the same actionable
`fcgi pool saturated` warn (`timeout_kind=concurrency`, `queue=disabled`).
Waiters are time-bounded; they are not an unlimited backlog.

## Sizing and the 503 envelope

Set the pool capacity from the PHP-FPM worker ceiling, not from the number of
incoming connections:

```text
max_concurrency >= php-fpm pm.max_children
max_connections  = max_concurrency   (default; max_connections <= max_concurrency)
```

Leave measured headroom above `pm.max_children`; a common starting point is
2× when the workload has bursts, but the real value must be measured. With 20
simultaneous 2 s requests and a 100 ms checkout budget, a pool of 16 is outside
the envelope and returns four 503s; the same workload with a pool of 40 returns
20×200 in the F6-WP-02 real PHP-FPM test. A larger `checkout_timeout_ms` only buys a bounded wait for a
connection; it cannot create PHP-FPM workers and must not be used to hide
underprovisioning.

Monitor the global `exyonq_fcgi_saturation_rejections_total` /
`exyonq_fcgi_responses_total{status="503"}` and the per-pool
`exyonq_fcgi_pool_saturation_rejections_total{pool=…}` /
`exyonq_fcgi_pool_responses_total{pool=…,status="503"}`. Any sustained increase
is an alert: check `pm.max_children`, PHP execution time, and that pool's
configured `max_concurrency`/`max_connections`. The named registry is bounded to
64 configured pools; the warn log carries the same pool name. See ADR-060.

## Reference

See checkout / wait / `PoolBusy` paths in
[`crates/exyonq-mod-fastcgi/src/conn_pool.rs`](../../crates/exyonq-mod-fastcgi/src/conn_pool.rs).
