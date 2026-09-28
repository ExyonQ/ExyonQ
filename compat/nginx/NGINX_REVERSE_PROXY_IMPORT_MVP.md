# NGINX reverse-proxy config import MVP

```text
DOCUMENT = compat/nginx/NGINX_REVERSE_PROXY_IMPORT_MVP.md
CONTRACT_ID = NGINX_REVERSE_PROXY_IMPORT_MVP_V1
STATUS = NORMATIVE_FOR_PROFILE
PROFILE = reverse-proxy-mvp
DEFAULT_MIGRATE_NGINX = UNCHANGED (Tier1/2 full importer)
STATIC_MVP = UNCHANGED
TOTAL_COMPAT_PROMISE = FORBIDDEN
PRODUCT_RUNTIME_CHANGED = NO
TRACKED = YES
```

## Purpose

Opt-in offline import of a **minimal reverse-proxy** NGINX subset into ExyonQ IR TOML.
This extends the migration path after `static-mvp`. It does **not** emulate NGINX and does
**not** claim general NGINX reverse-proxy compatibility.

## CLI

```bash
exyonqctl config migrate-nginx \
  --input nginx.conf \
  --profile reverse-proxy-mvp \
  --write --output exyonq.toml
```

Default profile remains the existing Tier1/2 importer (`--profile full`).
`static-mvp` is unchanged and still refuses `proxy_pass`.
`reverse-proxy-mvp` is fail-closed: any out-of-scope construct yields a structured diagnostic,
non-zero exit, and **no usable IR** (empty config body).

## Accepted grammar (explicit)

Top-level may be a bare `server { ... }` or an `http { server { ... } }` wrapper
containing only accepted server blocks.

Inside each `server`:

| Directive | Rule |
|-----------|------|
| `listen` | Required. Single address/port form mappable to ExyonQ `[[server]].listen`. |
| `server_name` | Optional. Literal names only (same limits as `static-mvp`). |
| `location PATH` | Exactly one. Simple **prefix** location only (no regex, no `=`, no `^~`, no named `@`). |
| `proxy_pass` | Required inside that location. Exactly `http://host:port` or `http://host:port/` (trailing slash on the authority only). |

Comments, ordinary whitespace, and newlines are accepted by the existing lexer.

## Mapping

Successful import emits ExyonQ IR (`config_version = 1`) with existing fields only:

- `[[server]]` listen + route list
- `[[route]]` with `match = { path = "<location>" }` and `upstream = "<name>"`
- `[[upstream]]` single-target `target = "http://host:port"` (legacy single member)

No new IR fields. No runtime product changes.

## Rejected (structured; no IR)

These **must** fail the `reverse-proxy-mvp` profile (diagnostic + exit ≠ 0, empty config):

- `upstream { ... }` blocks; named `proxy_pass` targets that require them
- `https://` upstreams; `unix:` sockets; variables (`$…`) in `proxy_pass`
- URI-bearing `proxy_pass` (path after authority), e.g. `http://127.0.0.1:3000/app`
- `proxy_set_header`, `proxy_http_version`, `proxy_buffering`, `proxy_cache`, custom
  timeouts/retries, WebSocket/`Upgrade`, `CONNECT`
- `grpc_pass`, `fastcgi_pass`, `uwsgi_pass`, `scgi_pass`
- `rewrite`, `return`, `if`, `map`, `include`, auth, TLS, load balancing
- location regex, named location, `^~`, `alias`, `try_files`
- `root` / `index` / static+proxy combinations in the same profile
- Multiple `location` blocks; unknown directives; ambiguous / partial Full-importer semantics

## Limits

- Offline only (`exyonq-compat-nginx` + `exyonqctl`). No nginx parse on the serve hot path.
- Does not change default `migrate-nginx` Tier1/2 behavior or `static-mvp`.
- Does not authorize runtime product changes.
- Darwin local serve of generated IR is iteration proof only; Linux Core Regression Gate remains
  the authority for CORE_* axes when that harness is present on an evidence host.

## Examples

### Accept

```nginx
server {
    listen 127.0.0.1:8080;
    server_name api.local;
    location /api/ {
        proxy_pass http://127.0.0.1:3000;
    }
}
```

### Reject

```nginx
location / {
    proxy_pass https://127.0.0.1:3000;
}
```

```nginx
upstream app { server 127.0.0.1:3000; }
location / { proxy_pass http://app; }
```

```nginx
location / {
    proxy_set_header Host $host;
    proxy_pass http://127.0.0.1:3000;
}
```
