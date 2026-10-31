# Apache directive catalog V1 (Phase 0)

**Status:** living inventory for migration — not a promise of runtime parity  
**Importer:** `exyonq-compat-apache` equivalent map (`IMPORTER_VERSION=0.4.11`, workspace `0.4.11`)  
**Taxonomy:** see product target §6.1

| Directive / construct | Classification | Importer | Notes |
| --- | --- | --- | --- |
| `Listen` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `[[server]].listen` (`0.0.0.0:port` if port-only) |
| `<VirtualHost>` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | One ExyonQ `[[server]]` + routes |
| `ServerName` | SUPPORTED_EXACT (intent) | yes | → `server_name` |
| `ServerAlias` | SUPPORTED_EXACT (intent) | yes | Merged into `server_name` list |
| `DocumentRoot` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → static `[[route]].root` path `/` (skipped when PHP-FPM owns `/`) |
| `DirectoryIndex` | SUPPORTED_DECLARED_DIFFERENCE | yes | First token only → `index` |
| `Alias` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | Extra static `[[route]]` (`match.path` + `root`) |
| `Redirect` / `Redirect permanent` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `route.redirect` (301/302) |
| `RewriteEngine On` + safe `RewriteRule ^/literal$ … [R=301\|302,L]` | SUPPORTED_DECLARED_DIFFERENCE | yes | Literal path only; regex/`RewriteCond` rejected |
| `SSLEngine on` + `SSLCertificateFile` / `SSLCertificateKeyFile` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `[[server]].tls` |
| `ProxyPass /path http://host:port/` (authority-only) | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `[[upstream]]` + `route.upstream` |
| `ProxyPassMatch` PHP / `FilesMatch` + `SetHandler` (`proxy:unix:…\|fcgi://…`) | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `[[fcgi_pool]]` + `route.fastcgi` |
| `ServerAlias` empty / missing `ServerName` | MANUAL_ACTION_REQUIRED | refuse usable IR | Require `ServerName` |
| Missing `DocumentRoot` (and no PHP/proxy-only site) | REJECTED_UNSUPPORTED | refuse | Blocker when no alternate root handler |
| `RewriteCond` / regex `RewriteRule` / `RedirectMatch` | REJECTED_UNSUPPORTED | reject+line | Use native htaccess overlay for WP front-controller |
| `ProxyPassReverse <local-path> <remote-url>` | MIGRATED_TO_EXYONQ_CONSTRUCT | yes | → `[[route.location_rewrite]]` (F3-R20); 1-arg `<Location>` form rejected |
| `ProxyPass` with upstream path / balancers / variables | REJECTED_UNSUPPORTED | reject+line | Same fail-closed policy as NGINX proxy MVP |
| `AuthType` / `AuthUserFile` / `Require` | REJECTED_UNSUPPORTED | reject+line | Auth surface absent |
| `ErrorDocument` | REJECTED_UNSUPPORTED | reject+line | No custom error-page IR |
| `Include` / `IncludeOptional` | REJECTED_UNSUPPORTED | reject+line | Expand in later cut |
| Unknown directive | REJECTED_UNSUPPORTED | reject+line | Never silent |

## Corpus A (PHP site) — planned, not closed

Minimum files for Gate A (future):

- multi-vhost + distinct docroots
- TLS vhost (SSL map landed in importer; dual-arch evidence still open)
- front-controller rewrite set (htaccess import path)
- PHP-FPM `ProxyPassMatch` or equivalent → `fcgi_pool` (importer map landed)

## Corpus B (app behind proxy) — planned

- `ProxyPass` keepalive upstream (authority-only map landed)
- WebSocket upgrade
- auth + rate limit (when product auth exists)
