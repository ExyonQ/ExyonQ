# ExyonQ Cache (WordPress)

Minimal **invalidation client** for ExyonQ L1 full-page cache (program phase **WC4**).

This plugin does **not** store pages, talk to Redis/Lux, or access ExyonQ memory. It only sends authenticated purge commands to the existing WC3 Unix purge socket.

## Requirements

| Item | Value |
|------|--------|
| PHP | ≥ 8.0 |
| WordPress | ≥ 6.0 |
| Multisite | **Not supported** (deferred — hooks and WP-CLI inactive) |
| ExyonQ | WC3 purge socket enabled |

## Configuration (`wp-config.php` only)

```php
define( 'EXYONQ_CACHE_PURGE_SOCKET', '/run/exyonq/cache-purge.sock' );
define( 'EXYONQ_CACHE_PURGE_TOKEN', 'replace-me' ); // never commit real secrets
define( 'EXYONQ_CACHE_SITE_ID', 123456789 ); // ExyonQ FPC site_id (must fit PHP_INT_MAX)
// optional:
// define( 'EXYONQ_CACHE_MAX_URLS_PER_EVENT', 20 );
// define( 'EXYONQ_CACHE_DEBUG', true ); // logs errors without token
```

Do **not** store the token in `wp_options`, the database, HTML, JS, or admin notices.

On the ExyonQ host:

```bash
export EXYONQ_CACHE_PURGE_SOCKET=/run/exyonq/cache-purge.sock
export EXYONQ_CACHE_PURGE_TOKEN='same-as-wp-config'
```

## Behaviour

- Content hooks (posts, comments, terms, theme/plugin/permalink) → purge URL list or site fallback
- Fail-open: socket/auth failures never break `save_post` / admin
- Per-request deduplication of identical purge ops
- Fan-out above `EXYONQ_CACHE_MAX_URLS_PER_EVENT` → `purge site`

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
docker run --rm -v "$PWD:/work" -w /work php:8.2-cli php tests/run-tests.php
```

## Out of scope (this plugin)

Redis object cache · Lux · L2 · WooCommerce advanced · tag purge · crawler/preload · Enterprise panel
