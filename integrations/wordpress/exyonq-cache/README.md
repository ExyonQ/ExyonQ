# ExyonQ Cache (WordPress)

Minimal **invalidation client** for ExyonQ L1 full-page cache (program phase **WC4**).

This plugin does **not** store pages, talk to Redis/Lux, or access ExyonQ memory. It only sends authenticated purge commands to the existing WC3 Unix purge socket.

## Requirements

| Item | Value |
| --- | --- |
| PHP | ≥ 8.0 |
| WordPress | ≥ 6.0 |
| Multisite | **Not supported** (deferred — hooks and WP-CLI inactive) |
| ExyonQ | WC3 purge socket enabled |

## Configuration (`wp-config.php` only)

```php
define( 'EXYONQ_CACHE_PURGE_SOCKET', '/run/exyonq/cache-purge.sock' );
define( 'EXYONQ_CACHE_PURGE_TOKEN', 'replace-me' ); // never commit real secrets
// Optional fallback only. ExyonQ publishes the real u64 id in <socket>.sites.
// define( 'EXYONQ_CACHE_SITE_ID', '12345678901234567890' );
// optional:
// define( 'EXYONQ_CACHE_MAX_URLS_PER_EVENT', 20 );
// define( 'EXYONQ_CACHE_DEBUG', true ); // logs errors without token
```

Do **not** store the token in `wp_options`, the database, HTML, JS, or admin notices.

The client reads, in order, the explicit constants above, then the files ExyonQ
publishes next to the socket: `cache-purge.token` and `<socket>.sites`. The site
index is authoritative because `EXYONQ_CACHE_SITE_ID` is a full u64 and a stale
constant must not break purge after a route rename. `EXYONQ_CACHE_ROUTE` defaults
to `wordpress`. If `EXYONQ_CACHE_PURGE_SOCKET` is omitted, the client looks for
`/run/exyonq*/cache-purge.sock` (including per-unit systemd directories).

On the ExyonQ host:

```bash
export EXYONQ_CACHE_PURGE_SOCKET=/run/exyonq/cache-purge.sock
export EXYONQ_CACHE_PURGE_TOKEN='same-as-wp-config'
```

When the purge socket starts, ExyonQ writes the token and site-index sidecars
automatically. The token file is mode 0640, the index is mode 0640 and the
socket is mode 0660; the PHP process must therefore run as the same user or in
the socket's group. If ExyonQ runs under systemd with
`RuntimeDirectory=/run/%p`, the socket is `/run/%p/cache-purge.sock`; the
WordPress process needs access to that per-unit directory (for example a shared
`www-data` group), not just `/run/exyonq`.

## Behaviour

- Content hooks (posts, comments, terms, theme/plugin/permalink) → purge URL list or site fallback
- Fail-open: socket/auth failures never break `save_post` / admin
- Per-request deduplication of identical purge ops
- Fan-out above `EXYONQ_CACHE_MAX_URLS_PER_EVENT` → `purge site`

## Admin panel

With the plugin active, **ExyonQ** appears in `wp-admin`. The panel shows the
resolved socket, whether it exists, whether a token and site index are visible,
the full decimal site id and the last purge result. It never prints the token.
The status labels are intentionally explicit when the WordPress container
cannot read `/etc/exyonq/wordpress.toml`; that condition means the panel cannot
verify the server profile, not necessarily that page caching is disabled.

## WP-CLI

```bash
wp exyonq-cache status
wp exyonq-cache purge-url https://example.com/hello/
wp exyonq-cache purge-site
wp exyonq-cache purge-generation 12
```

`status` never prints the token.

## Tests

```bash
docker run --rm -v "$PWD:/work" -w /work/integrations/wordpress/exyonq-cache \
  php:8.2-cli php tests/run-tests.php
```

## Out of scope (this plugin)

Redis object cache · Lux · L2 · WooCommerce advanced · tag purge · crawler/preload · Enterprise panel
