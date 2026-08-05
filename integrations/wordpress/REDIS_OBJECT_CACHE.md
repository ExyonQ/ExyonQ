# WordPress Redis object cache (WC5)

ExyonQ supports **external Redis** as the WordPress object-cache backend. This is **not** ExyonQ full-page cache and is **not** part of the HTTP dataplane.

| Layer | Owner |
|-------|--------|
| Full-page cache (HTML) | ExyonQ L1 + [exyonq-cache](./exyonq-cache/) purge plugin (WC4) |
| Object cache (PHP/DB objects) | Redis + [redis-cache](https://wordpress.org/plugins/redis-cache/) plugin (WC5) |

## Requirements

- PHP **phpredis** extension (preferred)
- Redis 7.x dedicated instance (single-site profile)
- WordPress ≥ 6.0 / PHP ≥ 8.0

## wp-config.php contract

```php
define( 'WP_CACHE', true );
define( 'WP_REDIS_HOST', 'redis' );           // or private endpoint
define( 'WP_REDIS_PORT', 6379 );
define( 'WP_REDIS_PASSWORD', '…' );         // from secret/env — never wp_options
define( 'WP_REDIS_PREFIX', 'exyonq-site-<id>:' );
define( 'WP_REDIS_DATABASE', 0 );
define( 'WP_REDIS_TIMEOUT', 1 );
define( 'WP_REDIS_READ_TIMEOUT', 1 );
define( 'WP_REDIS_CLIENT', 'phpredis' );
define( 'WP_REDIS_GRACEFUL', true ); // required: fail-open when Redis is down
```

Do **not** store Redis credentials in `wp_options`, HTML, JS, or public diagnostics.

## Install

```bash
wp plugin install redis-cache --activate
wp redis enable
wp redis status
```

## Lab harness (Plan 10A)

```bash
bash benchmarks/plan10a/wc5-redis-object-cache-e2e.sh
```

See `docs/architecture/cache/WC5_REDIS_OBJECT_CACHE_REPORT.md`.

## Experimental: Lux external (WC6)

Lux may be used as an **experimental** RESP backend with the **same** redis-cache + phpredis client (`WP_REDIS_HOST` pointing at Lux).

```text
LUX_EXTERNAL = EXPERIMENTAL_SUPPORTED_CANDIDATE
LUX_PRODUCT_SUPPORT = NOT_GRANTED
```

- Pin identity (commit/image digest) — never `:latest`. Lab: `benchmarks/plan10a/wc6-lux-compatibility-e2e.sh`.
- Report: `docs/architecture/cache/WC6_LUX_EXTERNAL_COMPATIBILITY_REPORT.md`.
- Redis remains the **supported** profile; do not replace WC5 with Lux in production claims.

## Out of scope

Lux embedded · Redis/Lux as FPC L2 · Cluster/Sentinel · Multisite · Redis/Lux in Rust/core
