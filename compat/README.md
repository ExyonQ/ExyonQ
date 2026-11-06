# Compatibility layer

Offline migration tools only. ExyonQ **never** reads Apache/NGINX/IIS config at runtime.

Importers live under `compat/` and are invoked via `exyonq-compat migrate`.

| Source | Crate | Notes |
| --- | --- | --- |
| NGINX | `nginx/` | Tier1/2 + static/proxy/fastcgi MVPs |
| Apache | `apache/` | **0.4.11** equivalent map (static, redirect, TLS, proxy, PHP-FPM); see `APACHE_STATIC_VHOST_MVP.md` |
| OpenLiteSpeed | `openlitespeed/` | `perClientConnLimit` → `[modules.ratelimit.client]` fragment via `exyonq-compat ols-throttle` (OL-03); not a full OLS importer |

Scope: [`docs/compat/compatibility-scope-v1.md`](../docs/compat/compatibility-scope-v1.md).
Apache catalog: [`docs/compat/APACHE_DIRECTIVE_CATALOG_V1.md`](../docs/compat/APACHE_DIRECTIVE_CATALOG_V1.md).
Release cut: [`docs/release/v0.4.11.md`](../docs/release/v0.4.11.md).
