# WordPress on ExyonQ

ExyonQ is the HTTP server. PHP-FPM is another process. The official image is
`FROM scratch` and does not contain PHP. SQLite or MySQL is also another
process, or a file on the volume. ExyonQ does not run PHP inside its binary.

## Which file to use

| File | Use |
| --- | --- |
| `conf/exyonq.toml` | Static site. Serves `html/`. |
| `conf/php-fpm.toml` | One PHP application. `root` plus `front_controller = "/index.php"`. |
| `conf/wordpress.toml` | WordPress. Static `/wp-includes` and `/wp-content`. PHP on `/`. |

`root` on a route is the directory of that prefix. In nginx that is `alias`,
not `root /var/www/html` for every location.

`/wp-content` and `/wp-includes` serve files. A `.php`, `.phtml`, `.phar`,
`.ht*` or dotfile on those routes is denied. `.well-known` stays reachable.
Plugin PHP is loaded by PHP-FPM from disk, not by downloading the file.
The database file `.ht.sqlite` is in that denied set.

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

`conf/wordpress.toml` sets `total_timeout_ms` to 120000. The product
profile used in tests stays at 30000. A plugin install or a core update
can exceed 30 seconds. A timeout from ExyonQ is 504. A dead PHP-FPM is
502.

## .htaccess

The stock WordPress file compiles. `RewriteBase` is accepted and not
applied. An unknown flag, including `E=HTTP_AUTHORIZATION:...`, is ignored
and the rest of the file is kept. `Authorization` is still forwarded to
PHP as `HTTP_AUTHORIZATION` when the request brings that header.
`DirectoryIndex` and `RewriteRule . /index.php [L]` are the rules the
front page uses.

The watcher recompiles only when a file named `.htaccess` changes. A
SQLite write does not republish the site.

## Page cache

`conf/wordpress.toml` turns `full_page_cache` on. Anonymous HTML is stored
for 300 seconds, at most 4 MB per object, 64 MB and 10000 entries in
namespace 4. A request that carries a cookie is not stored. The WordPress
image loads the purge plugin as a must-use plugin. When a post is
published, the plugin sends `purge site` on
`/run/exyonq/cache-purge.sock`.

ExyonQ opens that socket only when the cache is enabled and
`EXYONQ_CACHE_PURGE_SOCKET` is set. The entrypoint sets the path. ExyonQ
then writes `/run/exyonq/cache-purge.token` (mode 0600) and
`/run/exyonq/cache-purge.sock.sites`. The plugin reads those files. It
does not invent the site id. The scratch image does not run this cache.

## Containers

Static ExyonQ, scratch, port 8080:

```bash
docker build -f packaging/docker/Dockerfile -t exyonq:0.4.7 .
docker run --rm -p 8080:8080 exyonq:0.4.7
```

WordPress and SQLite, PHP-FPM plus that same static binary. Build the
scratch image first; this Dockerfile copies `/exyonq` out of it.

```bash
docker build -f packaging/docker/wordpress/Dockerfile -t exyonq-wordpress:0.4.7 .
docker run --rm -p 8080:8080 exyonq-wordpress:0.4.7
```

Published images, linux/amd64 and linux/arm64:

```bash
docker pull ghcr.io/exyonq/exyonq:0.4.7
docker pull ghcr.io/exyonq/exyonq-wordpress:0.4.7
```

Open `http://localhost:8080/` and finish the WordPress installer. The
database file is `wp-content/database/.ht.sqlite`. Fetching that URL, or
`/wp-content/db.php`, must not return the file.

After a WordPress core update, restart the container so preloaded CSS and
JavaScript are read again.

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
