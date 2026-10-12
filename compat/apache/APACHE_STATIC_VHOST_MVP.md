# Apache → ExyonQ equivalent importer (0.4.11)

**Product cut:** ExyonQ **0.4.11** (`IMPORTER_VERSION=0.4.11`)  
**CLI:** `exyonq-compat migrate --from apache --input httpd.conf --out exyonq.toml --report report.md`

Maps Apache constructs onto **existing** ExyonQ IR/runtime equivalents. Fail-closed: any `REJECTED_UNSUPPORTED` refuses TOML.

## Mapped equivalents (product already exists)

| Apache | ExyonQ IR |
|--------|-----------|
| `Listen` / `<VirtualHost *:port>` | `[[server]].listen` |
| `ServerName` / `ServerAlias` | `server_name` |
| `DocumentRoot` + `DirectoryIndex` | `[[route]]` static `root` / `index` |
| `Alias` | extra static `[[route]]` (`match.path` + `root`) |
| `Redirect` / `Redirect permanent` | `route.redirect` |
| `RewriteEngine` + safe `RewriteRule ^/path$ … [R=301\|302,L]` | `route.redirect` |
| `SSLEngine on` + cert/key | `[[server]].tls` |
| `ProxyPass /path http://host:port/` (authority-only) | `[[upstream]]` + `route.upstream` |
| `ProxyPassMatch` PHP / `FilesMatch` + `SetHandler proxy:unix:…\|fcgi://…` | `[[fcgi_pool]]` + `route.fastcgi` |

## Still rejected (no product equivalent yet)

| Apache | Why |
|--------|-----|
| `ErrorDocument` | No custom error-page IR |
| `AuthType` / `AuthUserFile` / `Require` | Auth module absent |
| `ProxyPassReverse` | No response `Location` rewrite |
| `RedirectMatch` / regex `RewriteRule` / `RewriteCond` | No regex match IR; use htaccess overlay for WP front-controller |
| `ProxyPass` with upstream path / balancers / variables | Same fail-closed policy as NGINX proxy MVP |
| `Include` | Expansion later |

`TOTAL_COMPAT_PROMISE = FORBIDDEN` until Gate A corpus closes on dual-arch evidence.
