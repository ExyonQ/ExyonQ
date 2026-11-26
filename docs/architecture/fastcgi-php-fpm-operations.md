# FastCGI / PHP-FPM operations

Applies to the module FastCGI path (`[[fcgi_pool]]` + `route.fastcgi`).

## Socket permissions and preflight

ExyonQ connects to PHP-FPM as its own service user. The socket must be
reachable by that user, directly or through a supplementary group:

```ini
; php-fpm pool
listen = /run/php/php-fpm.sock
listen.owner = www-data
listen.group = www-data
listen.mode = 0660
```

A `USR2` reload of the PHP-FPM master keeps the existing socket inode, so an
ownership fix applied only in the pool file is not visible until the master is
fully restarted and recreates the socket. A socket left as `root:root 0660`
makes every PHP request fail with `502`, while static routes keep working.

At every startup and structural reload ExyonQ probes each `[[fcgi_pool]]` as
the service user and logs the result:

```text
INFO  exyonq::fcgi: fcgi preflight ok pool=php85 endpoint=unix:/run/php/php-fpm.sock max_connections=16
WARN  exyonq::fcgi: fcgi preflight failed: unix:/run/php/php-fpm.sock (errno=EACCES, os_error=Some(13)): ...;
      socket permissions deny the ExyonQ user; set php-fpm listen.owner/listen.group ... pool=php85
      endpoint=unix:/run/php/php-fpm.sock errno="EACCES" os_error=Some(13) phase="preflight"
INFO  exyonq::fcgi: fcgi preflight summary pools=1 ok=0 failed=1
```

Preflight is advisory: it never aborts the generation, so static routes stay
available while the operator fixes PHP-FPM. A request that still cannot connect
logs the same `errno` at `phase="connect"` and returns `502`; a slow upstream
returns `504`, and pool saturation returns `503` (fail-fast, no unbounded
queue).

Offline check, useful in a deploy gate:

```bash
exyonqctl fastcgi check --config /etc/exyonq/exyonq.toml
# pool=php85 transport=unix endpoint=/run/php/php-fpm.sock max_concurrency=16 \
# max_connections=16 checkout_timeout_ms=5000 connect=ok
# FASTCGI_CHECK pools=1 ok=1 failed=0
```

It exits `0` only when every pool connects, `1` when any connect fails and `2`
when the config cannot be read or parsed. `--format json` emits the same rows
with `errno`, `os_error` and `detail` fields. It is a connection probe: it does
not execute a PHP script.

## Capacity sizing

| Setting | Meaning |
| --- | --- |
| `max_concurrency` | In-flight FastCGI requests admitted by the pool |
| `max_connections` | Backend sockets kept by the pool (defaults to `max_concurrency`) |
| `checkout_timeout_ms` | Bounded wait for a socket before `503` |

Set the pool from the PHP-FPM worker ceiling, not from the client connection
count:

```text
max_concurrency >= php-fpm pm.max_children
max_connections  = max_concurrency
```

Leave measured headroom above `pm.max_children`. A larger `checkout_timeout_ms`
only buys a bounded wait; it cannot create PHP-FPM workers. ExyonQ does not run
an unbounded nginx-style FastCGI queue. See
[`fcgi-saturation-policy-v1.md`](../architecture/fcgi-saturation-policy-v1.md).

## Per-pool diagnostics

Every configured pool exports:

```text
exyonq_fcgi_pool_inflight_current{pool="php85"}
exyonq_fcgi_pool_waiters_current{pool="php85"}
exyonq_fcgi_pool_checkout_timeouts_total{pool="php85"}
exyonq_fcgi_pool_connect_failures_total{pool="php85"}
exyonq_fcgi_pool_saturation_rejections_total{pool="php85"}
exyonq_fcgi_pool_responses_total{pool="php85",status="200|502|503|504"}
```

The global `exyonq_fcgi_*` series remain available for existing dashboards.
The named registry is capped at 64 configured pools; excess names share
`pool="__overflow__"`.

## Response body limit

The cross-cutting module pipeline collects response bodies so modules can
inspect or transform them. Its ceiling is configurable:

```toml
[modules]
# Default 4194304 (4 MiB); accepted range 1024..=1073741824.
response_body_limit_bytes = 16777216
```

- A response whose declared size (`Content-Length` or the body size hint) is
  **larger** than the ceiling is not collected. It streams through untouched,
  and modules observe status and headers only, as with SSE. Body-rewriting
  filters (compression, `sub_filter`) do not apply to that response.
- A body of **unknown** length that exceeds the ceiling is rejected with
  `413` and a body naming the configured limit. The log carries `limit`,
  `observed`, `route` and `upstream`.
- Large public assets should normally stream (leave the default) or be served
  from a dedicated static/CDN route. Raise the limit only when a module must
  inspect a body that size.

This removes the historical fixed 4 MiB failure where an 11 MiB static asset
returned `500 module error: response body exceeds module processing limit`
even on a static route when any module was enabled.
