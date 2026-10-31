# FastCGI saturation policy (Phase 1 / C2)

**Status:** Phase 1 product policy  
**Owner crate:** `crates/exyonq-mod-fastcgi/src/conn_pool.rs`

## Default behavior

ExyonQ does **not** implement an unbounded NGINX-style FastCGI wait queue.

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
deadline. Waiters are time-bounded; they are not an unlimited backlog.

## Reference

See checkout / wait / `PoolBusy` paths in
[`crates/exyonq-mod-fastcgi/src/conn_pool.rs`](../../crates/exyonq-mod-fastcgi/src/conn_pool.rs).
