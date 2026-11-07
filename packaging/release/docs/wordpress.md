# WordPress on ExyonQ

Applies to **v0.7.0**.

ExyonQ is the HTTP server. PHP-FPM is another process. The official server
image is Wolfi (Chainguard `glibc-dynamic`) and does not contain PHP. The
WordPress image is separate: Wolfi `php-8.5-fpm` (PHP 8.5.12) plus the `exyonq`
binary copied from that image. SQLite or MySQL is also another process, or a
file on the volume.
ExyonQ does not run PHP inside its binary. On first boot the WordPress image
creates `wp-config.php` and fills the eight salts locally with PHP's CSPRNG;
the container does not need outbound network access for that step.

## Which file to use

| File | Use |
| --- | --- |
| `packaging/docker/default.toml` | Official server image. Static page on `0.0.0.0:8080`. `config_version` 1. |
| `packaging/release/conf/wordpress.toml` | WordPress image. `config_version` 2. Static `/wp-includes` and `/wp-content`. PHP on `/`. |

This tree does not ship `conf/exyonq.toml` or `conf/php-fpm.toml`. The
validated, copyable example is `packaging/release/conf/wordpress.toml`;
`scripts/gates/config-examples-validate.sh` copies it to a temporary path and
runs `exyonq validate` against the real binary. The same file is the source of
the route shapes below.

A PHP application uses **separate routes**: static prefixes with `root` and
one catch-all route with `fastcgi`. A route cannot set both `root` and
`fastcgi`; that is a conflicting action and `exyonq validate` rejects it with
`EXY-CONFIG-0011`. Do not copy a single-route nginx location shape into TOML.
The catch-all route may use `htaccess = "overlay"` (as WordPress does) or an
explicit `front_controller = "index.php"`. The front controller is not a
property of a `root` route.

`root` on a route is the directory of that prefix. In nginx that is `alias`,
not `root /var/www/html` for every location.

`/wp-content` and `/wp-includes` serve files. A `.php`, `.phtml`, `.phar`,
`.ht*` or dotfile on those routes is denied. `.well-known` stays reachable.
Plugin PHP is loaded by PHP-FPM from disk, not by downloading the file.
The database file is in that denied set.

The `/` route uses `htaccess = "overlay"`; the stock WordPress `.htaccess`
provides the front-controller rewrite. Without an overlay, set
`front_controller = "index.php"` explicitly. This is the equivalent of
`try_files $uri $uri/ /index.php?$args`: existing files and directory indexes
win, and every other URI runs `/index.php` with `PATH_INFO` and the original
query string.

## Static asset caching

Both static asset routes set a 30-day, immutable `Cache-Control` for every
MIME type served under that prefix:

```toml
[route.cache_headers.by_mime."*"]
max_age = 2592000
immutable = true
```

WordPress appends a version query to enqueued assets, so the 30-day window is
intentional. This is a route-level header policy, not a page-cache TTL; PHP
execution and `/wp-config.php` are not affected. The production file uses the
inline equivalent so the policy stays attached to each static route.

## Blocking `wp-config.php`

The production config contains a terminal route with no backend:

```toml
[[route]]
name = "wp-config-deny"
match = { path = "/wp-config.php" }
access = { deny = ["all"] }
```

`match.path` uses a segment boundary, so this returns `403` for
`/wp-config.php`, `/wp-config.php/` and `/wp-config.php/nested`, without
executing PHP. It does not block `/wp-config.php.bak` or `/wp-config.phpx`;
those continue to normal route resolution. Do not use `exact = true` for this
security boundary.

The `/` route executes an existing `.php`. An existing directory runs its
`index.php`, or serves `index.html` when that is the only index. Every
other URL goes to `/index.php`. The original query string is kept. The
client URI is not rewritten when a directory index runs, so `/wp-admin/`
executes `/wp-admin/index.php` instead of the site front controller.

## PHP-FPM

In the official PHP image the listen that applies is `docker.conf`
(`listen = 9000`), not the commented lines in `www.conf`. The WordPress
image in `packaging/docker/wordpress/` replaces that listen with
`/run/php/php-fpm.sock`.

The official ExyonQ binary uses uid 10001. PHP-FPM in the PHP image uses
`www-data` (uid 33). A socket mode `0660` works when both sides share a
user or a group. The WordPress image runs ExyonQ as `www-data`. If they
do not share a user or a group, PHP answers 502.

`expose_php = Off` is set in that image so responses do not carry
`X-Powered-By: PHP/...`.

`packaging/release/conf/wordpress.toml` sets `total_timeout_ms` to 120000. The product
profile used in tests stays at 30000. A plugin install or a core update
can exceed 30 seconds. A timeout from ExyonQ is 504. A dead PHP-FPM is
502.

## Capacity and 503

ExyonQ admits `max_concurrency` FastCGI requests per pool and keeps at most
`max_connections` sockets (defaults to `max_concurrency`). Size the pool to the
PHP-FPM worker ceiling and leave measured headroom:

```text
max_concurrency >= php-fpm pm.max_children
max_connections  = max_concurrency
```

This is a bounded fail-fast policy, not an unbounded nginx queue. With 20
simultaneous 2 s requests and a 100 ms checkout budget, a pool of 16 returns
four 503s; a pool of 40 returns 20×200 in the F6-WP-02 real PHP-FPM test.
`checkout_timeout_ms` can absorb a short burst, but raising it does not add
PHP-FPM workers. Alert on sustained
`exyonq_fcgi_saturation_rejections_total` /
`exyonq_fcgi_pool_saturation_rejections_total{pool=…}`, or
`exyonq_fcgi_responses_total{status="503"}` /
`exyonq_fcgi_pool_responses_total{pool=…,status="503"}`; the warn log names the
pool and includes `queue=disabled`.

## .htaccess

The stock WordPress file compiles. `RewriteBase` is accepted and not
applied. An unknown flag, including `E=HTTP_AUTHORIZATION:...`, is ignored
and the rest of the file is kept. `Authorization` is still forwarded to
PHP as `HTTP_AUTHORIZATION` when the request brings that header.
`DirectoryIndex` and `RewriteRule . /index.php [L]` are the rules the
front page uses.

The watcher recompiles only when a file named `.htaccess` changes. A
SQLite write does not republish the site.

## Migrating from nginx and rolling back

Treat ExyonQ as a replacement for the nginx `server` block, not as a
byte-for-byte emulation. Keep nginx installed and its configuration untouched
until the direct path has passed the gate.

1. Keep the production listener and nginx upstream unchanged. Run ExyonQ on a
   spare port against a **copy** of the docroot, the same PHP-FPM socket and a
   disposable copy of the database. Do not point the public listener at it yet.
2. If the docroot has no stock WordPress `.htaccess` (a common nginx
   `try_files` setup), use `front_controller = "index.php"` on the catch-all
   FastCGI route. Existing files and directory indexes still win; every other
   URI runs `/index.php` with the original query string. Do not use
   `htaccess = "overlay"` without the stock file: the route has no rewrite and
   returns 404.
3. Gate the copy on `/`, a category, a post, a missing URL (404), `/feed/`,
   `/wp-json/`, CSS/JS and uploads, `/wp-admin/`, `/wp-login.php`,
   `/wp-config.php` (403), Brotli, immutable asset caching and at least 20
   concurrent PHP requests. Compare the decoded HTML with the current
   production response; compressed sizes are useful, but the decoded bytes
   are the content contract.
4. Cut over only after that evidence is recorded and the release artifact is
   the version being advertised. Stop the public ExyonQ process and restore the
   previous nginx listener (or the previous reverse-proxy unit) to roll back.
   The docroot, database and nginx configuration are unchanged, so rollback
   needs no content or data migration.
5. Do not change the production footer or release post to a new ExyonQ version
   until the published artifact for that version has passed the gate.

The F6-WP-04 harness is `scripts/e2e/f6-wp04-migration-content-e2e.sh`; it
copies a real tree and DB into a temporary directory, runs PHP-FPM and ExyonQ
without nginx, and records the HTTP, Brotli and concurrency results.

## Page cache

`packaging/release/conf/wordpress.toml` turns `full_page_cache` on. Anonymous HTML is stored
for 300 seconds, at most 4 MB per object, 64 MB and 10000 entries in
namespace 4. A request that carries a cookie is not stored. The WordPress image ships the purge plugin at `wp-content/plugins/exyonq-cache`.
It is a normal plugin: activate it under Plugins (single-site). The image does
not install a must-use plugin. When a post is published, the active plugin
sends `purge site` on `/run/exyonq/cache-purge.sock`. A comment purges that
post's URL. A menu change purges the tag `menu`. A URL that is not in the cache
does not empty the site.

ExyonQ opens that socket only when the cache is enabled and
`EXYONQ_CACHE_PURGE_SOCKET` is set. The entrypoint sets the path and a
persistent `EXYONQ_CACHE_PURGE_TOKEN` in `wp-config.php`; the plugin reads that
constant. The token survives a container restart. The plugin does not invent
the site id. The plain server image does not run this cache.

## Containers

Static ExyonQ, port 8080:

```bash
docker build -f packaging/docker/Dockerfile -t exyonq:0.7.0 .
docker run --rm -p 8080:8080 exyonq:0.7.0
```

WordPress and SQLite, PHP-FPM plus the server binary. Build the server
image first; this Dockerfile copies `/usr/local/bin/exyonq` out of it.

```bash
docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.7.0 .
docker run --rm -p 8080:8080 exyonq-wordpress:0.7.0
```

Published images, linux/amd64 and linux/arm64:

```bash
docker pull ghcr.io/exyonq/exyonq:0.7.0
docker pull ghcr.io/exyonq/exyonq-wordpress:0.7.0
```

Open `http://localhost:8080/` and finish the WordPress installer. The
database file is `wp-content/database/db.sqlite` — one fixed, predictable
file. Set `EXYONQ_WP_DB_PATH` to move it (for example onto a dedicated
volume outside the docroot). Fetching that URL, or `/wp-content/db.php`,
must not return the file.

After a WordPress core update, the next request for a replaced CSS or
JavaScript file reads the new bytes. An unchanged file stays preloaded.

## Access log

`[logging.access] enabled = true` writes one line per request on stdout,
including the status. The response also carries `x-request-id`. PHP-FPM
receives that header as `HTTP_X_REQUEST_ID` when the client sent
`X-Request-Id`. ExyonQ adds its own id on the response.

## What this is

ExyonQ serves files and speaks FastCGI, in the same family as nginx.
It is not a shared-hosting Apache, not OpenLiteSpeed, and not HAProxy.
The `.exy` syntax does not express FastCGI or this WordPress file. The
production configuration is TOML.
