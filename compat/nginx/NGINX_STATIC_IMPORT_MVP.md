# NGINX static config import MVP

```text
DOCUMENT = compat/nginx/NGINX_STATIC_IMPORT_MVP.md
CONTRACT_ID = NGINX_STATIC_IMPORT_MVP_V1
STATUS = NORMATIVE_FOR_PROFILE
PROFILE = static-mvp
DEFAULT_MIGRATE_NGINX = UNCHANGED (Tier1/2 full importer)
TOTAL_COMPAT_PROMISE = FORBIDDEN
PRODUCT_RUNTIME_CHANGED = NO
TRACKED = YES
LOCAL_MIRROR = docs/config/nginx-static-import-mvp.md (gitignored; not authoritative for Git)
```

## Purpose

Opt-in offline import of a **static hosting** NGINX subset into ExyonQ IR TOML.
This is the first migration step from NGINX. It does **not** emulate NGINX and does
**not** claim general NGINX compatibility.

## CLI

```bash
exyonqctl config migrate-nginx \
  --input nginx.conf \
  --profile static-mvp \
  --write --output exyonq.toml
```

Default profile remains the existing Tier1/2 importer (`--profile full`).
`static-mvp` is fail-closed: any out-of-scope construct yields a structured diagnostic,
non-zero exit, and **no usable IR** (empty config body).

## Accepted grammar (explicit)

Top-level may be a bare `server { ... }` or an `http { server { ... } }` wrapper
containing only accepted server blocks.

Inside each `server`:

| Directive | Rule |
|-----------|------|
| `listen` | Required. Single address/port form mappable to ExyonQ `[[server]].listen`. |
| `server_name` | Optional. Literal names only (no regex `~`, no `*`-wildcards beyond catch-all `_` ignored). |
| `root` | Required. Document root for static files: at server level and/or inside optional `location /` (at least one). |
| `index` | Optional; defaults to `index.html` in IR when absent. |
| `location /` | Optional. Only exact prefix `location /` (no regex, no `^~`, no other paths). Nested body may only carry `root` / `index`, or be empty. |

Comments, ordinary whitespace, and newlines are accepted by the existing lexer.

## Mapping

Successful import emits ExyonQ IR (`config_version = 1`) with:

- `[[server]]` listen + route list
- `[[route]]` with `match = { path = "/" }`, `root`, `index`

## Rejected (structured; no IR)

These **must** fail the `static-mvp` profile (diagnostic + exit ≠ 0, empty config):

- `proxy_pass`, `fastcgi_pass`, `uwsgi_pass`, `grpc_pass`
- `rewrite`, `return`, `if`, `map`, `upstream`, `split_clients`, lua, third-party modules
- `include` (even when the full importer could expand it)
- TLS / certificates, HTTP/2/3, cache, rate limit, WAF, auth, load balancing
- Multiple distinct `location` behaviors or any `location` other than `/`
- Any unknown directive
- Ambiguous or partial mappings that the full importer might mark LOSSY/PARTIAL

## Limits

- Offline only (`exyonq-compat-nginx` + `exyonqctl`). No nginx parse on the serve hot path.
- Does not change default `migrate-nginx` Tier1/2 behavior or claims.
- Does not authorize runtime product changes.
- Darwin local serve of generated IR is iteration proof only; Linux Core Regression Gate remains the authority for CORE_* axes when that harness is present on an evidence host.
