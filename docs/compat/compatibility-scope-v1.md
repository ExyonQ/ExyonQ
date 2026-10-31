# Compatibility scope V1

**Status:** active for ExyonQ 0.4.11 cut  
**Rule:** offline importers only — ExyonQ never reads Apache/NGINX config at runtime.

## Declared sources

| Source | Crate / CLI | Maturity |
| --- | --- | --- |
| NGINX | `exyonq-compat-nginx` / `exyonq-compat migrate --from nginx` | Tier1/2 + static/proxy/fastcgi MVPs |
| Apache httpd | `exyonq-compat-apache` / `exyonq-compat migrate --from apache` | **Equivalent map** (static + redirect + TLS + proxy + PHP-FPM) |

## Apache equivalent importer (Gate A precursor — not Gate A)

In scope for successful TOML emit when product IR already exists:

- Global `Listen` / `<VirtualHost …>`
- `ServerName`, optional `ServerAlias`
- `DocumentRoot` + optional `DirectoryIndex` → static route
- `Alias` → extra static routes
- `Redirect` / safe literal `RewriteRule … [R=…]` → redirect routes
- Patterned `RewriteRule` (`$1..$9`, `[L]`/`[R=…]`/`[NC]`) → `match.regex` + `rewrite` rule (ADR-048 / NG-02)
- Minimal `RewriteCond` (`%{REQUEST_URI|HTTP_HOST|QUERY_STRING|HTTP:Header}`, AND) → `rewrite.when`
- `SSLEngine` + cert/key → `[[server]].tls`
- Authority-only `ProxyPass` → `[[upstream]]` + proxy route
- `ProxyPassReverse <local-path> <remote-url>` → `[[route.location_rewrite]]` (F3-R20: `Location`/`Content-Location`/`URI`)
- PHP-FPM via `ProxyPassMatch` / `FilesMatch`+`SetHandler` → `[[fcgi_pool]]` + fastcgi route

Still **REJECTED_UNSUPPORTED** (blocks clean migration until product grows):

- `ErrorDocument`, `AuthType` / `Require`
- Advanced rewrite: `RewriteCond [OR]` / `!` / unlisted vars / flags `QSA|PT|END|…`; `RedirectMatch`
- `ProxyPass` with upstream path, balancers, variables
- `Include` / `IncludeOptional`
- Unknown directives (never silent)

Product communication until Gate A closes: never claim “full Apache substitution”. See [APACHE_DIRECTIVE_CATALOG_V1.md](APACHE_DIRECTIVE_CATALOG_V1.md) and [compat/apache/APACHE_STATIC_VHOST_MVP.md](../../compat/apache/APACHE_STATIC_VHOST_MVP.md).

## TOTAL_COMPAT_PROMISE

**FORBIDDEN** for Apache and NGINX until the corresponding gate corpus passes on Netcup AMD64 and Oracle ARM64.
