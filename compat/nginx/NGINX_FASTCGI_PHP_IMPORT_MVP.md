# NGINX FastCGI/PHP config import MVP

```text
DOCUMENT = compat/nginx/NGINX_FASTCGI_PHP_IMPORT_MVP.md
CONTRACT_ID = NGINX_FASTCGI_PHP_IMPORT_MVP_V1
STATUS = NORMATIVE_FOR_PROFILE
PROFILE = fastcgi-php-mvp
DEFAULT_MIGRATE_NGINX = UNCHANGED (Tier1/2 full importer)
STATIC_MVP = UNCHANGED
REVERSE_PROXY_MVP = UNCHANGED
TOTAL_COMPAT_PROMISE = FORBIDDEN
PRODUCT_RUNTIME_CHANGED = NO
WORDPRESS_COMPAT_PROMISE = FORBIDDEN
TRACKED = YES
```

## Purpose

Opt-in offline import of a **minimal FastCGI/PHP** NGINX subset into ExyonQ IR TOML.
Maps one explicit PHP PREFIX location + TCP `fastcgi_pass` + `root` onto existing
`[[route]].fastcgi` / `[[fcgi_pool]]` fields. Does **not** emulate NGINX, does **not**
claim WordPress/pretty-permalink/`try_files` front-controller parity, and does **not**
change Full / `static-mvp` / `reverse-proxy-mvp` policy (Full still blocks TCP FastCGI).

## CLI

```bash
exyonqctl config migrate-nginx \
  --input nginx.conf \
  --profile fastcgi-php-mvp \
  --write --output exyonq.toml
```

Fail-closed: any out-of-scope construct → structured diagnostic, exit ≠ 0, **empty** IR body.

## Accepted grammar (explicit)

Top-level may be a bare `server { ... }` or `http { server { ... } }` containing only
accepted server blocks.

Inside each `server`:

| Directive | Rule |
|-----------|------|
| `listen` | Required. Single form mappable to `[[server]].listen` (port, `*:port`, `0.0.0.0:port`, `127.0.0.1:port`, IPv4/IPv6 literals). Not `localhost`. |
| `server_name` | Optional. Literal names only. |
| `root` | Required. Absolute filesystem path → `fcgi_pool.document_root`. |
| `location PATH` | Exactly one. Simple **prefix** only. `PATH` must be an explicit PHP script path (starts with `/`, ends with `.php`, no regex/`=`/`^~`/`@`). |
| `fastcgi_pass` | Required inside that location. Exactly `IP:port` or `[IPv6]:port` parsable as `SocketAddr` (TCP). No `unix:`, no hostname DNS, no `$vars`, no named upstream. |
| `fastcgi_param` | Optional, at most the exact pair: `SCRIPT_FILENAME $document_root$fastcgi_script_name`. Omit params entirely is OK. Any other param/form is rejected. |

## Mapping

Successful import emits ExyonQ IR (`config_version = 1`) with existing fields only:

- `[[server]]` listen + route list (+ optional `server_name`)
- `[[route]]` `match.path` = location path, `fastcgi = "<pool>"`, `index =` absent
- `[[fcgi_pool]]` `address = "IP:port"`, `transport = "tcp"`, `document_root = "<root>"`

SCRIPT_FILENAME authority remains ExyonQ (`document_root` + request path). The optional
`fastcgi_param` line is accepted only when it matches that inherent form; it is not stored
as a free-form param table.

No new IR fields. No FastCGI runtime / schema changes.

## Rejected (structured; no IR)

- `index`, `fastcgi_index` (FastCGI IR has no index append; static `route.index` must not be used)
- `include` (including `fastcgi_params` / `fastcgi.conf`)
- `unix:` sockets; hostname `fastcgi_pass`; `$variables`; `upstream { }`
- Other `fastcgi_param` / `fastcgi_*` (buffering, cache, timeouts, split_path_info, …)
- `try_files`, `rewrite`, `return`, `if`, `alias`, `proxy_pass`
- Regex / exact / preferential / named locations; multi-location; nested location
- TLS, auth, unknown directives; ambiguous static+proxy+FastCGI mixes

## Limits

- Offline only (`exyonq-compat-nginx` + `exyonqctl`). No nginx parse on the serve hot path.
- Does not change Full TCP FastCGI block policy.
- Not WordPress product compatibility; a `/index.php` PREFIX may be used to hit a PHP
  front controller script that happens to be WordPress-shaped, nothing more.
- Darwin local serve is iteration only; Linux Core Regression Gate remains authority for CORE_* axes.

## Examples

### Accept

```nginx
server {
    listen 127.0.0.1:8080;
    server_name php.local;
    root /var/www/html;
    location /index.php {
        fastcgi_pass 127.0.0.1:9000;
        fastcgi_param SCRIPT_FILENAME $document_root$fastcgi_script_name;
    }
}
```

### Reject

```nginx
location ~ \.php$ { fastcgi_pass 127.0.0.1:9000; }
```

```nginx
location /index.php {
    include fastcgi_params;
    fastcgi_pass unix:/run/php/php-fpm.sock;
}
```

```nginx
index index.php;
fastcgi_index index.php;
```
